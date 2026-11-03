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
use crate::lighting::parser::*;
use std::time::Duration;

#[test]
fn test_sequence_definition_and_reference() {
    let content = r#"
sequence "color_cycle" {
    @0.000
    front_wash: static, color: "red", duration: 5s
    
    @2.000
    front_wash: static, color: "green", duration: 5s
    
    @4.000
    front_wash: static, color: "blue", duration: 5s
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
    front_wash: static, color: "red", duration: 5s
    
    @1.000
    front_wash: static, color: "blue", duration: 5s
}

show "Test Show" {
    @5.000
    back_wash: static, color: "green", duration: 5s
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
    front_wash: static, color: "red", duration: 5s
    
    @1/3
    front_wash: static, color: "green", duration: 5s
    
    @2/1
    front_wash: static, color: "blue", duration: 5s
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
    front_wash: static, color: "red", duration: 5s
    
    @2/1
    front_wash: static, color: "blue", duration: 5s
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
    front_wash: static, color: "red", duration: 5s
    
    @1/3
    front_wash: static, color: "green", duration: 5s
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
fn test_sequence_with_comment_after_loop() {
    // Test that comments on a new line after sequence loop directive work
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "test" {
    @1/1
    front_wash: static color: "red", duration: 5s
}

show "Test" {
    @0.000
    sequence "test", loop: 3
    # Some comment
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse sequence with comment after loop directive: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");

    // Should have 3 cues (1 cue per iteration × 3 iterations)
    assert_eq!(show.cues.len(), 3);
}

#[test]
fn test_layer_command_with_comment_after_parameter() {
    // Test that comments on a new line after layer command parameters work
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @0.000
    clear(layer: foreground)
    # Some comment
    
    @1.000
    freeze(layer: background)
    # Another comment
    
    @2.000
    master(layer: midground, intensity: 0.5)
    # Yet another comment
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse layer commands with comments: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");

    // Should have 3 cues
    assert_eq!(show.cues.len(), 3);

    // First cue should have clear command with foreground layer
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.layer_commands.len(), 1);
    let clear_cmd = &first_cue.layer_commands[0];
    assert_eq!(
        clear_cmd.command_type,
        crate::lighting::parser::LayerCommandType::Clear
    );
    assert_eq!(
        clear_cmd.layer,
        Some(crate::lighting::effects::EffectLayer::Foreground)
    );

    // Second cue should have freeze command
    let second_cue = &show.cues[1];
    assert_eq!(second_cue.layer_commands.len(), 1);
    let freeze_cmd = &second_cue.layer_commands[0];
    assert_eq!(
        freeze_cmd.command_type,
        crate::lighting::parser::LayerCommandType::Freeze
    );
    assert_eq!(
        freeze_cmd.layer,
        Some(crate::lighting::effects::EffectLayer::Background)
    );

    // Third cue should have master command with intensity
    let third_cue = &show.cues[2];
    assert_eq!(third_cue.layer_commands.len(), 1);
    let master_cmd = &third_cue.layer_commands[0];
    assert_eq!(
        master_cmd.command_type,
        crate::lighting::parser::LayerCommandType::Master
    );
    assert_eq!(
        master_cmd.layer,
        Some(crate::lighting::effects::EffectLayer::Midground)
    );
    assert!(master_cmd.intensity.is_some());
    assert!((master_cmd.intensity.unwrap() - 0.5).abs() < 0.001);
}

#[test]
fn test_effect_parameter_with_comment_after() {
    // Test that comments on a new line after effect parameters work
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @0.000
    front_wash: static, color: "red", layer: foreground, duration: 5s
    # Comment after effect
    
    @1.000
    back_wash: cycle, speed: 2beats, direction: forward, duration: 10s
    # Another comment
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse effects with comments: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");

    // Should have 2 cues
    assert_eq!(show.cues.len(), 2);

    // First cue should have static effect
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.effects.len(), 1);
    let effect = &first_cue.effects[0];
    assert_eq!(
        effect.layer,
        Some(crate::lighting::effects::EffectLayer::Foreground)
    );

    // Second cue should have cycle effect
    let second_cue = &show.cues[1];
    assert_eq!(second_cue.effects.len(), 1);
}

#[test]
fn test_sequence_looping_finite() {
    let content = r#"
sequence "simple_sequence" {
    @0.000
    front_wash: static, color: "red", duration: 5s
    
    @1.000
    front_wash: static, color: "blue", duration: 5s
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

    // Sequence duration = max(0+5, 1+5) = 6s per iteration
    // First iteration: 0s, 1s
    assert_eq!(show.cues[0].time, Duration::from_secs(0));
    assert_eq!(show.cues[1].time, Duration::from_secs(1));

    // Second iteration: 6s, 7s
    assert_eq!(show.cues[2].time, Duration::from_secs(6));
    assert_eq!(show.cues[3].time, Duration::from_secs(7));

    // Third iteration: 12s, 13s
    assert_eq!(show.cues[4].time, Duration::from_secs(12));
    assert_eq!(show.cues[5].time, Duration::from_secs(13));

    // All effects should be marked with sequence name
    for cue in &show.cues {
        for effect in &cue.effects {
            assert_eq!(effect.sequence_name, Some("simple_sequence".to_string()));
        }
    }

    // First iteration's first cue should NOT have stop_sequences
    assert!(
        show.cues[0].stop_sequences.is_empty(),
        "First iteration should not stop anything"
    );

    // Second iteration's first cue should stop "simple_sequence"
    assert_eq!(
        show.cues[2].stop_sequences,
        vec!["simple_sequence".to_string()],
        "Second iteration should stop previous iteration's effects"
    );

    // Third iteration's first cue should stop "simple_sequence"
    assert_eq!(
        show.cues[4].stop_sequences,
        vec!["simple_sequence".to_string()],
        "Third iteration should stop previous iteration's effects"
    );

    // Non-first cues in each iteration should NOT have stop_sequences
    assert!(show.cues[1].stop_sequences.is_empty());
    assert!(show.cues[3].stop_sequences.is_empty());
    assert!(show.cues[5].stop_sequences.is_empty());
}

#[test]
fn test_sequence_looping_infinite() {
    let content = r#"
sequence "infinite_sequence" {
    @0.000
    front_wash: static, color: "red", duration: 5s
    
    @1.000
    front_wash: static, color: "blue", duration: 5s
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
    // Sequence duration = max(0+5, 1+5) = 6s per iteration
    assert_eq!(show.cues[0].time, Duration::from_secs(0));
    assert_eq!(show.cues[1].time, Duration::from_secs(1));
    assert_eq!(show.cues[2].time, Duration::from_secs(6)); // Second iteration starts at 6s
    assert_eq!(show.cues[3].time, Duration::from_secs(7));
}

#[test]
fn test_sequence_looping_once() {
    let content = r#"
sequence "once_sequence" {
    @0.000
    front_wash: static, color: "red", duration: 5s
    
    @1.000
    front_wash: static, color: "blue", duration: 5s
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
    front_wash: static, color: "red", duration: 5s
    
    @1.000
    front_wash: static, color: "blue", duration: 5s
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
    front_wash: static, color: "red", duration: 5s
}

sequence "seq2" {
    @0.000
    back_wash: static, color: "blue", duration: 5s
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

    // Find cues at 5 seconds with stop_sequences
    // With 5s duration effects, iteration boundaries also occur at 5s,
    // so there may be multiple cues at 5s with stop_sequences.
    // Collect all stop_sequences from all cues at 5s.
    let cues_at_5: Vec<_> = show
        .cues
        .iter()
        .filter(|c| c.time == Duration::from_secs(5))
        .collect();
    assert!(!cues_at_5.is_empty(), "Should have cues at 5 seconds");

    let all_stops: std::collections::HashSet<_> = cues_at_5
        .iter()
        .flat_map(|c| c.stop_sequences.iter())
        .collect();
    assert!(
        all_stops.contains(&"seq1".to_string()),
        "Should stop seq1 at 5s, got stops: {:?}",
        all_stops
    );
    assert!(
        all_stops.contains(&"seq2".to_string()),
        "Should stop seq2 at 5s, got stops: {:?}",
        all_stops
    );
}

#[test]
fn test_nested_sequences() {
    let content = r#"
sequence "base_pattern" {
    @0.000
    front_wash: static, color: "red", duration: 5s
    
    @1.000
    front_wash: static, color: "blue", duration: 5s
}

sequence "complex_pattern" {
    @0.000
    sequence "base_pattern"
    back_wash: static, color: "green", duration: 5s
    
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
    front_wash: static, color: "red", duration: 5s
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
    all_wash: static, color: "white", duration: 5s
}

sequence "verse" {
    @1/1
    sequence "verse-start", loop: 1
    @13/1
    all_wash: static, color: "red", duration: 5s
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
fn test_sequence_with_fractional_measure_hold_time() {
    // Test that fractional measure durations in hold_time are calculated correctly
    // (hold_time in measures is resolved as a measure duration, fractions included)
    // Since parse_light_shows only returns shows, not sequences directly,
    // we need to test the sequence by referencing it in a show
    // We'll verify the hold_time calculation by using the sequence in a show
    let show_content = r##"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "riff-e" {
    @1/1
    all_wash: static, color: "#B5C637", layer: background, blend_mode: replace, duration: 5s

    @1/3
    all_wash: static, color: "#8A0303", layer: background, blend_mode: replace, hold_time: 1.5measures
}

show "Test" {
    @0.000
    sequence "riff-e"
}
"##;

    let show_result = parse_light_shows(show_content);
    assert!(
        show_result.is_ok(),
        "Failed to parse show with sequence: {:?}",
        show_result.err()
    );

    let shows_with_sequence = show_result.unwrap();
    let show = shows_with_sequence
        .get("Test")
        .expect("Show 'Test' should exist");

    // The sequence should be expanded into the show's cues
    // We should have 2 cues from the sequence
    assert!(
        show.cues.len() >= 2,
        "Show should have at least 2 cues from sequence, got {}",
        show.cues.len()
    );

    // Find the cue with the effect that has hold_time
    let cue_with_hold_time = show
        .cues
        .iter()
        .find(|cue| cue.effects.iter().any(|effect| effect.hold_time.is_some()));

    let (sequence_cue, second_effect) = if let Some(cue) = cue_with_hold_time {
        let effect = cue
            .effects
            .iter()
            .find(|e| e.hold_time.is_some())
            .expect("Should find effect with hold_time");
        (cue, effect)
    } else {
        panic!("Should find a cue with an effect that has hold_time");
    };

    // Second effect has hold_time: 1.5measures
    let hold_time = second_effect
        .hold_time
        .expect("Second effect should have hold_time");

    // At 120 BPM in 4/4: 1.5 measures = 6 beats = 3.0 seconds
    let expected_hold_time = Duration::from_secs_f64(3.0);
    assert!(
        (hold_time.as_secs_f64() - expected_hold_time.as_secs_f64()).abs() < 0.001,
        "hold_time should be 3.0s (1.5 measures at 120 BPM in 4/4), got {}s",
        hold_time.as_secs_f64()
    );

    // Verify the effect's total duration (should be just hold_time since no up_time or down_time)
    let total_duration = second_effect.total_duration();
    assert!(
        (total_duration.as_secs_f64() - expected_hold_time.as_secs_f64()).abs() < 0.001,
        "Total duration should be 3.0s, got {}s",
        total_duration.as_secs_f64()
    );

    // Verify the effect completes at the expected time
    // Second effect starts at measure 1, beat 3 (1.0s) and has hold_time of 1.5 measures (3.0s)
    // So second effect completes at: 1.0s + 3.0s = 4.0s
    // At 120 BPM in 4/4: 4.0s = 8 beats = 2 measures
    let effect_start_time = sequence_cue.time;
    let total_duration = second_effect.total_duration();
    let effect_completion_time = effect_start_time + total_duration;
    let expected_completion_time = Duration::from_secs_f64(4.0);
    assert!(
        (effect_completion_time.as_secs_f64() - expected_completion_time.as_secs_f64()).abs()
            < 0.001,
        "Effect should complete at 4.0s (2 measures from sequence start), got {}s",
        effect_completion_time.as_secs_f64()
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

#[test]
fn test_sequence_tempo_rescaling_at_expansion() {
    // A sequence parsed at the global base tempo (110 BPM) should have its cue
    // times rescaled when expanded at a point where the tempo is 160 BPM.
    //
    // The sequence has two cues: @1/1 (beat 0) and @1/2 (beat 1).
    // At 110 BPM, @1/2 is at 0.545s. At 160 BPM, @1/2 should be at 0.375s.
    //
    // The sequence is 1 beat long (perpetual effects, duration = last_cue - first_cue).
    // At 110 BPM that's 0.545s. At 160 BPM it should be 0.375s per iteration.
    //
    // We expand the sequence 4 times starting at measure 5 (after a tempo change
    // at measure 3 from 110 to 160 BPM).
    let content = r#"
tempo {
    bpm: 110
    time_signature: 4/4
    changes: [
        @3/1 { bpm: 160 }
    ]
}

sequence "test_seq" {
    @1/1
    front_wash: static, color: "red", duration: 5s

    @1/2
    front_wash: static, color: "blue", duration: 5s
}

show "Test Show" {
    @5/1
    sequence "test_seq", loop: 4
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Should have 8 cues (2 cues per iteration x 4 iterations)
    assert_eq!(show.cues.len(), 8);

    // At 110 BPM, 4/4 time:
    //   Measure 1: beat 0, time 0.0s
    //   Measure 2: beat 4, time 4*60/110 = 2.1818s
    //   Measure 3: beat 8, time 8*60/110 = 4.3636s  (tempo changes to 160 here)
    //   Measure 4: 4.3636 + 4*60/160 = 4.3636 + 1.5 = 5.8636s
    //   Measure 5: 5.8636 + 4*60/160 = 5.8636 + 1.5 = 7.3636s
    let expected_base_time = 8.0 * 60.0 / 110.0 + 8.0 * 60.0 / 160.0;
    let base_time = show.cues[0].time.as_secs_f64();
    assert!(
        (base_time - expected_base_time).abs() < 0.001,
        "First cue should be at measure 5 ({:.4}s), got {:.4}s",
        expected_base_time,
        base_time
    );

    // The sequence was parsed at 110 BPM. Its internal cue relative times:
    //   @1/1 = 0.0s (0 beats), @1/2 = 0.5454s (1 beat)
    //   effects have 5s duration, so sequence duration = max(0+5, 0.5454+5) = 5.5454s
    //   In beats at 110 BPM: 5.5454 * (110/60) = ~10.167 beats
    //
    // When expanded at measure 5 (160 BPM), each beat = 60/160 = 0.375s.
    // Iteration spacing = 10.167 beats * 0.375s/beat = ~3.8125s
    // Second cue within each iteration = 1 beat at 160 BPM = 0.375s offset
    let beat_at_160 = 60.0 / 160.0; // 0.375s
    let duration_internal = 5.0 + 60.0 / 110.0; // 5.5454s (max completion time at 110 BPM)
    let duration_beats = duration_internal * (110.0 / 60.0); // convert to beats
    let iteration_spacing = duration_beats * beat_at_160; // convert beats to seconds at 160 BPM

    // Check iteration spacing: first cue of each iteration
    for i in 0..4 {
        let cue_time = show.cues[i * 2].time.as_secs_f64();
        let expected = expected_base_time + i as f64 * iteration_spacing;
        assert!(
            (cue_time - expected).abs() < 0.01,
            "Iteration {} first cue should be at {:.4}s, got {:.4}s",
            i,
            expected,
            cue_time
        );
    }

    // Check the second cue in each iteration (1 beat later at 160 BPM)
    for i in 0..4 {
        let cue_time = show.cues[i * 2 + 1].time.as_secs_f64();
        let expected = expected_base_time + i as f64 * iteration_spacing + beat_at_160;
        assert!(
            (cue_time - expected).abs() < 0.01,
            "Iteration {} second cue should be at {:.4}s, got {:.4}s",
            i,
            expected,
            cue_time
        );
    }
}

#[test]
fn test_sequence_tempo_rescaling_same_tempo() {
    // When the expansion tempo matches the sequence's parse tempo, cue times
    // should be unchanged (no rescaling effect).
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "same_tempo_seq" {
    @1/1
    front_wash: static, color: "red", duration: 5s

    @1/3
    front_wash: static, color: "green", duration: 5s

    @2/1
    front_wash: static, color: "blue", duration: 5s
}

show "Test Show" {
    @5/1
    sequence "same_tempo_seq"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    assert_eq!(show.cues.len(), 3);

    // At 120 BPM, measure 5 starts at 4 measures * 4 beats * 60/120 = 8.0s
    // Sequence cues relative: @1/1 = 0.0s, @1/3 = 1.0s (2 beats), @2/1 = 2.0s (4 beats)
    // Since expansion tempo (120 BPM) matches sequence parse tempo (120 BPM),
    // the rescaled times should match the original relative times.
    let base = 8.0;
    assert!(
        (show.cues[0].time.as_secs_f64() - base).abs() < 0.001,
        "First cue at {:.4}s, expected {:.4}s",
        show.cues[0].time.as_secs_f64(),
        base
    );
    assert!(
        (show.cues[1].time.as_secs_f64() - (base + 1.0)).abs() < 0.001,
        "Second cue at {:.4}s, expected {:.4}s",
        show.cues[1].time.as_secs_f64(),
        base + 1.0
    );
    assert!(
        (show.cues[2].time.as_secs_f64() - (base + 2.0)).abs() < 0.001,
        "Third cue at {:.4}s, expected {:.4}s",
        show.cues[2].time.as_secs_f64(),
        base + 2.0
    );
}

/// #529: a sequence parsed at the opening tempo and expanded in a faster
/// section resolves its beat and measure durations at the expansion tempo,
/// as its cue times are, and leaves durations in seconds alone. Esaweg's
/// shape: 110 BPM at the top, 160 from bar 3.
#[test]
fn test_sequence_durations_resolve_at_the_expansion_tempo() {
    let content = r#"
tempo {
    start: 0.0s
    bpm: 110
    time_signature: 4/4
    changes: [
        @3/1 { bpm: 160 }
    ]
}

sequence "hit" {
    @1/1
    front_wash: static, color: "orange", duration: 3beats
    back_wash: static, color: "white", duration: 1measure
    side_wash: static, color: "red", duration: 0.5s
    @1/4
    front_wash: static, color: "orange", duration: 1beat
}

show "Test" {
    @3/1
    sequence "hit"
}
"#;

    let shows = parse_light_shows(content).expect("show parses");
    let show = shows.get("Test").unwrap();
    let tm = show.tempo_map.as_ref().unwrap();
    let bar3 = tm.measure_to_time_with_offset(3, 1.0, 0, 0.0).unwrap();
    let first = show
        .cues
        .iter()
        .find(|c| c.time == bar3)
        .expect("the sequence's first cue lands on bar 3");

    let duration_of = |group: &str| {
        first
            .effects
            .iter()
            .find(|e| e.groups == vec![group.to_string()])
            .map(|e| e.effect_type.duration().as_secs_f64())
            .unwrap()
    };
    // 160 BPM: a beat is 0.375 s, a bar 1.5 s. At 110 they were 1.636 s and
    // 2.182 s, longer than the 1.5 s bar the sequence plays in.
    assert!(
        (duration_of("front_wash") - 1.125).abs() < 1e-6,
        "3 beats: {}",
        duration_of("front_wash")
    );
    assert!(
        (duration_of("back_wash") - 1.5).abs() < 1e-6,
        "1 measure: {}",
        duration_of("back_wash")
    );
    assert!(
        (duration_of("side_wash") - 0.5).abs() < 1e-9,
        "0.5 s: {}",
        duration_of("side_wash")
    );

    // The sequence's anchor cue on beat 4 is on the 160 grid, so a 3-beat
    // effect from beat 1 ends exactly on it.
    let anchor = bar3 + Duration::from_secs_f64(1.125);
    assert!(
        show.cues
            .iter()
            .any(|c| (c.time.as_secs_f64() - anchor.as_secs_f64()).abs() < 1e-6),
        "the @1/4 cue should be at {anchor:?}"
    );
}

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

#[test]
fn test_sequence_length_sets_the_loop_period() {
    // Three and a half beats of content: without a length, each pass would
    // last 3.5 beats and the loop would drift off the bar.
    let content = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "centre-beats", length: 1measure {
    @1/1
    front_wash: static, color: "blue", duration: 1beat
    @1/3
    front_wash: static, color: "white", duration: 1.5beats
}

sequence "inner", length: 2beats {
    @1/1
    back_wash: static, color: "red", duration: 1beat
}

sequence "outer" {
    @1/1
    sequence "inner", loop: 2
    @2/1
    side_wash: static, color: "green", duration: 1beat
}

show "Test" {
    @2/1
    sequence "centre-beats", loop: 3
    @6/1
    sequence "outer"
}
"#;

    let shows = parse_light_shows(content).expect("show parses");
    let show = shows.get("Test").unwrap();
    let tm = show.tempo_map.as_ref().unwrap();
    let at = |bar: u32, beat: f64| tm.measure_to_time_with_offset(bar, beat, 0, 0.0).unwrap();
    let starts_of = |group: &str| -> Vec<Duration> {
        show.cues
            .iter()
            .filter(|c| {
                c.effects
                    .iter()
                    .any(|e| e.groups == vec![group.to_string()])
            })
            .map(|c| c.time)
            .collect()
    };

    assert_eq!(
        starts_of("front_wash"),
        vec![
            at(2, 1.0),
            at(2, 3.0),
            at(3, 1.0),
            at(3, 3.0),
            at(4, 1.0),
            at(4, 3.0)
        ],
        "each pass starts on its own bar"
    );
    // Nested, the inner sequence's length still sets its period: two passes
    // of 2 beats each, not of the 1 beat its effect lasts.
    assert_eq!(starts_of("back_wash"), vec![at(6, 1.0), at(6, 3.0)]);
    assert_eq!(starts_of("side_wash"), vec![at(7, 1.0)]);
}

#[test]
fn test_sequence_length_shorter_than_its_effects() {
    let content = r#"
sequence "hold", length: 2s {
    @0.000
    front_wash: static, color: "blue", duration: 5s
}

show "Test" {
    @1.000
    sequence "hold", loop: 3
}
"#;

    let shows = parse_light_shows(content).expect("show parses");
    let show = shows.get("Test").unwrap();
    let times: Vec<Duration> = show.cues.iter().map(|c| c.time).collect();
    assert_eq!(
        times,
        vec![
            Duration::from_secs(1),
            Duration::from_secs(3),
            Duration::from_secs(5)
        ]
    );
    // Each later pass stops the one before it, cutting the 5 s effect at
    // the pass boundary.
    assert!(show.cues[1].stop_sequences.contains(&"hold".to_string()));
    assert!(show.cues[2].stop_sequences.contains(&"hold".to_string()));
}

#[test]
fn test_sequence_length_rejects_cues_past_it_and_zero() {
    let past = r#"
sequence "late", length: 1s {
    @0.000
    front_wash: static, color: "blue", duration: 1s
    @1.000
    front_wash: static, color: "red", duration: 1s
}

show "Test" {
    @0.000
    sequence "late"
}
"#;
    let err = parse_light_shows(past).expect_err("a cue at the length is rejected");
    let message = err.to_string();
    assert!(
        message.contains("late") && message.contains("1s"),
        "names the sequence and its length: {message}"
    );

    let zero = past.replace("length: 1s", "length: 0s");
    let err = parse_light_shows(&zero).expect_err("a zero length is rejected");
    assert!(err.to_string().contains("greater than zero"), "{err}");

    let musical = past.replace("length: 1s", "length: 1measure");
    let err = parse_light_shows(&musical).expect_err("musical length needs a tempo");
    assert!(err.to_string().contains("tempo"), "{err}");
}

/// The times of a show's cues, in seconds, rounded to the millisecond.
fn cue_secs(content: &str, show: &str) -> Vec<f64> {
    let shows = parse_light_shows(content).unwrap();
    shows[show]
        .cues
        .iter()
        .map(|c| (c.time.as_secs_f64() * 1000.0).round() / 1000.0)
        .collect()
}

#[test]
fn a_sequence_counts_its_bars_at_one_tempo_whatever_the_songs_changes() {
    // The song slows to 60 at bar 2. A sequence is played anywhere, so its
    // own bar 3 is 8 beats after its bar 1 -- not wherever the song's bar 3
    // falls. Played from the song's bar 1 those 8 beats are 4 at 120 (2s) and
    // 4 at 60 (4s). The sequence's bar 3 used to be timed through the song's
    // change and read back at 120 as 12 beats, putting it at 10s.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @2/1 { bpm: 60 }
    ]
}

sequence "s" {
    @1/1
    front_wash: static, color: "red", duration: 1beat

    @3/1
    front_wash: static, color: "blue", duration: 1beat
}

show "Show" {
    @1/1
    sequence "s"
}
"#;
    assert_eq!(cue_secs(content, "Show"), vec![0.0, 6.0]);
}

#[test]
fn a_nested_sequence_follows_the_tempo_map_of_the_sequence_it_is_in() {
    // "outer" slows to 60 at its own bar 2 and plays "inner" (4 beats a
    // pass) twice from its bar 3: at its beats 8 and 12. Played in a show at
    // a steady 120 those are 4s and 6s. The nested passes used to be laid
    // end to end at the outer sequence's starting tempo, and read back
    // through its slower bars as beats 12 and 16 (6s and 8s).
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "inner" {
    @1/1
    front_wash: static, color: "red", duration: 4beats
}

sequence "outer" {
    tempo {
        bpm: 120
        time_signature: 4/4
        changes: [
            @2/1 { bpm: 60 }
        ]
    }

    @1/1
    back_wash: static, color: "blue", duration: 1beat

    @3/1
    sequence "inner", loop: 2
}

show "Show" {
    @1/1
    sequence "outer"
}
"#;
    assert_eq!(cue_secs(content, "Show"), vec![0.0, 4.0, 6.0]);
}

#[test]
fn an_inline_loop_follows_a_tempo_change_inside_it() {
    // Two beats a pass from bar 1, which is at 120; the song slows to 60 at
    // bar 2, so the third and fourth passes take a second a beat. The passes
    // used to be a fixed length in seconds: 0, 1, 2 and 3s.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @2/1 { bpm: 60 }
    ]
}

show "Show" {
    @1/1
    loop {
        @1/1
        front_wash: static, color: "red", duration: 2beats
    }, repeats: 4
}
"#;
    assert_eq!(cue_secs(content, "Show"), vec![0.0, 1.0, 2.0, 4.0]);
}

#[test]
fn an_inline_loop_in_seconds_keeps_its_seconds_after_a_tempo_change() {
    // The song slows to 60 at bar 9 (16s); bar 10 is at 20s. A loop there
    // written in seconds is timed at the tempo where it is (60), so its
    // cues stay half a second apart and a pass lasts its second. Timed
    // through the song's map from its bar 1 it came out a second apart.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @9/1 { bpm: 60 }
    ]
}

show "Show" {
    @10/1
    loop {
        @0.000
        front_wash: static, color: "red", duration: 500ms
        @0.500
        front_wash: static, color: "blue", duration: 500ms
    }, repeats: 2
}
"#;
    assert_eq!(cue_secs(content, "Show"), vec![20.0, 20.5, 21.0, 21.5]);
}

#[test]
fn an_inline_loop_in_seconds_after_an_early_change_keeps_its_seconds() {
    // The song slows to 60 at bar 2; bar 5 is at 2s + 12s. The body's
    // seconds are seconds at 60: a second apart, four a pass.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @2/1 { bpm: 60 }
    ]
}

show "Show" {
    @5/1
    loop {
        @0.000
        front_wash: static, color: "red", duration: 1s
        @1.000
        front_wash: static, color: "blue", duration: 1s
        @2.000
        front_wash: static, color: "red", duration: 1s
        @3.000
        front_wash: static, color: "blue", duration: 1s
    }, repeats: 2
}
"#;
    assert_eq!(
        cue_secs(content, "Show"),
        vec![14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 21.0]
    );
}

#[test]
fn an_inline_loop_in_beats_follows_the_map_where_each_pass_plays() {
    // Two beats a pass from bar 2 (2s), at 120 until the song slows to 60
    // at bar 3 (4s): the fourth pass, from beat 10, starts 2s after the
    // third.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @3/1 { bpm: 60 }
    ]
}

show "Show" {
    @2/1
    loop {
        @1/1
        front_wash: static, color: "red", duration: 2beats
    }, repeats: 4
}
"#;
    assert_eq!(cue_secs(content, "Show"), vec![2.0, 3.0, 4.0, 6.0]);
}

/// A song at 120 that slows to 60 at bar 9 (16s).
const SLOWS_AT_NINE: &str = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @9/1 { bpm: 60 }
    ]
}
"#;

fn close_to(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 0.002, "{got:?} vs {want:?}");
    }
}

#[test]
fn an_inline_loop_in_seconds_keeps_its_seconds_across_a_tempo_change() {
    // From bar 8 (14s), half a second a cue, a second a pass, through the
    // change at 16s: every pass keeps its second.
    let content = format!(
        r#"{SLOWS_AT_NINE}
show "Show" {{
    @8/1
    loop {{
        @0.000
        front_wash: static, color: "red", duration: 500ms
        @0.500
        front_wash: static, color: "blue", duration: 500ms
    }}, repeats: 6
}}
"#
    );
    let want: Vec<f64> = (0..12).map(|k| 14.0 + 0.5 * k as f64).collect();
    close_to(&cue_secs(&content, "Show"), &want);
}

#[test]
fn an_inline_loop_in_seconds_keeps_its_seconds_through_a_ramp() {
    // A ramp from 120 to 60 over four beats from bar 3 (4s); bar 3 beat 3
    // is two beats into it, at 4s + 1.117s. The loop's half seconds stay
    // half seconds while the ramp runs.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @3/1 { bpm: 60, transition: 4 }
    ]
}

show "Show" {
    @3/3
    loop {
        @0.000
        front_wash: static, color: "red", duration: 500ms
        @0.500
        front_wash: static, color: "blue", duration: 500ms
    }, repeats: 4
}
"#;
    let start = 4.0 + (2.0 - 2.5f64.sqrt()) / 0.375;
    let want: Vec<f64> = (0..8).map(|k| start + 0.5 * k as f64).collect();
    close_to(&cue_secs(content, "Show"), &want);
}

#[test]
fn an_inline_loop_in_measures_counts_the_time_signature_where_it_plays() {
    // 3/4 from bar 3 (4s): a pass of one measure is three beats, 1.5s.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @3/1 { time_signature: 3/4 }
    ]
}

show "Show" {
    @3/1
    loop {
        @1/1
        front_wash: static, color: "red", duration: 1measure
    }, repeats: 4
}
"#;
    close_to(&cue_secs(content, "Show"), &[4.0, 5.5, 7.0, 8.5]);
}

#[test]
fn an_offset_inside_an_inline_loop_is_sized_at_the_loops_tempo() {
    // At 60 from bar 9, a loop at bar 10 (20s) whose first cue offsets
    // the rest by a measure: four beats, 4s at the loop's tempo, so its
    // bar-2 cue is 8 beats in and a pass (to that cue's beat) 9 beats.
    let content = format!(
        r#"{SLOWS_AT_NINE}
show "Show" {{
    @10/1
    loop {{
        @1/1
        front_wash: static, color: "red", duration: 1beat
        offset 1 measures
        @2/1
        front_wash: static, color: "blue", duration: 1beat
    }}, repeats: 2
}}
"#
    );
    close_to(&cue_secs(&content, "Show"), &[20.0, 28.0, 29.0, 37.0]);
}

#[test]
fn an_inline_loop_in_seconds_after_a_start_offset_keeps_its_seconds() {
    // Bar 1 at 2s, so bar 2 at 4s.
    let content = r#"
tempo {
    start: 2s
    bpm: 120
    time_signature: 4/4
}

show "Show" {
    @2/1
    loop {
        @0.000
        front_wash: static, color: "red", duration: 500ms
        @0.500
        front_wash: static, color: "blue", duration: 500ms
    }, repeats: 2
}
"#;
    close_to(&cue_secs(content, "Show"), &[4.0, 4.5, 5.0, 5.5]);
}

#[test]
fn a_sequence_in_seconds_keeps_its_seconds_across_a_tempo_change() {
    // As an inline loop's: a body written wholly in seconds keeps its
    // seconds on every pass, wherever it plays.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "s" {{
    @0.000
    front_wash: static, color: "red", duration: 500ms
    @0.500
    front_wash: static, color: "blue", duration: 500ms
}}

show "Show" {{
    @8/1
    sequence "s", loop: 4
}}
"#
    );
    let want: Vec<f64> = (0..8).map(|k| 14.0 + 0.5 * k as f64).collect();
    close_to(&cue_secs(&content, "Show"), &want);
}

#[test]
fn a_sequence_in_beats_follows_the_map_across_a_tempo_change() {
    // Two beats a pass from bar 8 (14s): 1s a pass at 120 until 16s, 2s a
    // pass at 60 after.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "s" {{
    @1/1
    front_wash: static, color: "red", duration: 2beats
}}

show "Show" {{
    @8/1
    sequence "s", loop: 4
}}
"#
    );
    close_to(&cue_secs(&content, "Show"), &[14.0, 15.0, 16.0, 18.0]);
}

#[test]
fn a_loop_at_times_with_effects_in_beats_counts_in_beats() {
    // P1: the cue is at a time, but its effect lasts a beat, so the body is
    // in beats: a pass a beat, half a second until the change at 16s, a
    // second after.
    let content = format!(
        r#"{SLOWS_AT_NINE}
show "Show" {{
    @8/1
    loop {{
        @0.000
        front_wash: static, color: "red", duration: 1beat
    }}, repeats: 8
}}
"#
    );
    close_to(
        &cue_secs(&content, "Show"),
        &[14.0, 14.5, 15.0, 15.5, 16.0, 17.0, 18.0, 19.0],
    );
}

#[test]
fn a_sequence_at_times_playing_one_in_beats_counts_in_beats() {
    // P2: "outer"'s one cue is at a time, but it plays "inner", which is in
    // beats: "outer" lays out as "inner" would played directly.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "inner" {{
    @1/1
    front_wash: static, color: "red", duration: 1beat
    @1/2
    front_wash: static, color: "blue", duration: 1beat
}}

sequence "outer" {{
    @0.000
    sequence "inner"
}}

show "Show" {{
    @8/1
    sequence "outer", loop: 4
}}
"#
    );
    close_to(
        &cue_secs(&content, "Show"),
        &[14.0, 14.5, 15.0, 15.5, 16.0, 17.0, 18.0, 19.0],
    );
}

#[test]
fn a_sequence_at_times_with_a_loop_in_beats_counts_in_beats() {
    // P3: an inline loop of beat cues inside a sequence whose cue is at a
    // time: the sequence is in beats.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "s" {{
    @0.000
    loop {{
        @1/1
        front_wash: static, color: "red", duration: 1beat
    }}, repeats: 2
}}

show "Show" {{
    @8/1
    sequence "s", loop: 4
}}
"#
    );
    close_to(
        &cue_secs(&content, "Show"),
        &[14.0, 14.5, 15.0, 15.5, 16.0, 17.0, 18.0, 19.0],
    );
}

#[test]
fn a_sequence_at_times_with_effects_in_beats_counts_in_beats() {
    // P4: cues at 0 and 0.25s (half a beat at 120), each lasting a beat: a
    // pass of a beat and a half, following the change at 16s.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "s" {{
    @0.000
    front_wash: static, color: "red", duration: 1beat
    @0.250
    front_wash: static, color: "blue", duration: 1beat
}}

show "Show" {{
    @8/1
    sequence "s", loop: 4
}}
"#
    );
    close_to(
        &cue_secs(&content, "Show"),
        &[14.0, 14.25, 14.75, 15.0, 15.5, 15.75, 16.5, 17.0],
    );
}

/// The times of a show's cues with effects, in seconds, to the millisecond.
fn effect_cue_secs(content: &str, show: &str) -> Vec<f64> {
    let shows = parse_light_shows(content).unwrap();
    shows[show]
        .cues
        .iter()
        .filter(|c| !c.effects.is_empty())
        .map(|c| (c.time.as_secs_f64() * 1000.0).round() / 1000.0)
        .collect()
}

#[test]
fn a_sequence_in_seconds_keeps_its_seconds_inside_one_in_beats() {
    // "b" is in seconds; "a", in beats, plays it from its bar 1. Played at
    // bar 9 (16s, where it is 60), "b"'s anchor is placed in beats with
    // "a", and its cues keep their half seconds from it: as "b" played
    // there directly.
    let b = r#"
sequence "b" {
    @0.000
    front_wash: static, color: "red", duration: 500ms
    @0.500
    front_wash: static, color: "blue", duration: 500ms
}
"#;
    let alone = format!(
        r#"{SLOWS_AT_NINE}{b}
show "Show" {{
    @9/1
    sequence "b", loop: 2
}}
"#
    );
    let nested = format!(
        r#"{SLOWS_AT_NINE}{b}
sequence "a" {{
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "b", loop: 2
}}

show "Show" {{
    @9/1
    sequence "a"
}}
"#
    );
    let want = [16.0, 16.5, 17.0, 17.5];
    close_to(&cue_secs(&alone, "Show"), &want);
    close_to(&cue_secs(&nested, "Show"), &want);
}

#[test]
fn an_inline_loop_in_seconds_keeps_its_seconds_inside_a_sequence_in_beats() {
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "a" {{
    @1/1
    back_wash: static, color: "green", duration: 1beat
    loop {{
        @0.000
        front_wash: static, color: "red", duration: 500ms
        @0.500
        front_wash: static, color: "blue", duration: 500ms
    }}, repeats: 2
}}

show "Show" {{
    @9/1
    sequence "a"
}}
"#
    );
    close_to(&cue_secs(&content, "Show"), &[16.0, 16.5, 17.0, 17.5]);
}

#[test]
fn an_offset_on_a_cue_holding_only_an_empty_loop_still_counts_in_beats() {
    // The offset makes the body count in beats whether or not the cue that
    // issues it leaves a cue of its own: 2s at 120 is four beats, placed
    // at 60 from bar 9 as five beats on.
    let body = |extra: &str| {
        format!(
            r#"{SLOWS_AT_NINE}
sequence "s" {{
    @0.000
    back_wash: static, color: "green", duration: 100ms
    @0.100
    offset 1 measures
    {extra}
    @0.500
    front_wash: static, color: "red", duration: 500ms
}}

show "Show" {{
    @9/1
    sequence "s"
}}
"#
        )
    };
    let with_loop = effect_cue_secs(&body("loop { }, repeats: 1"), "Show");
    let without = effect_cue_secs(&body(""), "Show");
    close_to(&without, &[16.0, 21.0]);
    close_to(&with_loop, &without);
}

#[test]
fn a_cue_holding_only_an_empty_loop_still_marks_where_its_body_starts() {
    // Its offset counts, and so does its place: the body starts at 0, so
    // the red cue, 2.5s (five beats) in, is five beats after bar 9.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "s" {{
    @0.000
    offset 1 measures
    loop {{ }}, repeats: 1
    @0.500
    front_wash: static, color: "red", duration: 500ms
}}

show "Show" {{
    @9/1
    sequence "s"
}}
"#
    );
    close_to(&effect_cue_secs(&content, "Show"), &[21.0]);
}

#[test]
fn a_shows_sequences_are_laid_out_as_they_play() {
    // The layout the web UI draws is the parse the player plays: the same
    // cues, by the cue and reference they are played from, and where the
    // last pass ends (two beats a pass from bar 8: 14, 15, then 16, 18, to
    // 20).
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "s" {{
    @1/1
    front_wash: static, color: "red", duration: 2beats
}}

show "Show" {{
    @1/1
    back_wash: static, color: "blue", duration: 1s

    @8/1
    sequence "s", loop: 4
}}
"#
    );
    let layouts = crate::lighting::parser::layout_sequence_calls(&content, None)
        .unwrap()
        .shows;
    assert_eq!(layouts.len(), 1);
    assert_eq!(layouts[0].name, "Show");
    let calls = &layouts[0].calls;
    assert_eq!(calls.len(), 1);
    assert_eq!(
        (calls[0].cue, calls[0].call, calls[0].name.as_str()),
        (1, 0, "s")
    );
    let times: Vec<f64> = calls[0].cues.iter().map(|c| c.time.as_secs_f64()).collect();
    close_to(&times, &[14.0, 15.0, 16.0, 18.0]);
    assert!((calls[0].end.as_secs_f64() - 20.0).abs() < 0.002);
    // As the show plays them.
    let played: Vec<f64> = cue_secs(&content, "Show")[1..].to_vec();
    close_to(&played, &times);
}

#[test]
fn a_pass_lasts_until_its_content_ends_where_it_plays() {
    // P2: "b", in beats, plays "c", in seconds (a second a pass, three
    // passes): played at bar 9 (60), "c"'s last pass ends at 19s, and so
    // does "b"'s pass -- where "c" ends where it lands (its `length:`
    // included, as played straight), not at the 120 "b" was written at.
    let content = format!(
        r#"{SLOWS_AT_NINE}
sequence "c", length: 1s {{
    @0.000
    front_wash: static, color: "red", duration: 200ms
}}

sequence "b" {{
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "c", loop: 3
}}

show "Show" {{
    @9/1
    sequence "b", loop: 2
}}
"#
    );
    close_to(
        &effect_cue_secs(&content, "Show"),
        &[16.0, 17.0, 18.0, 19.0, 20.0, 21.0],
    );
}

#[test]
fn a_cue_landing_past_its_passs_length_does_not_play() {
    // P3: "b" is a measure a pass, played at 240: a second. "s", in
    // seconds, has a cue 1.5s in, which lands after the next pass has
    // stopped this one: it does not play, as a cue written past a length
    // cannot.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @9/1 { bpm: 240 }
    ]
}

sequence "s" {
    @0.000
    front_wash: static, color: "red", duration: 200ms
    @1.500
    front_wash: static, color: "blue", duration: 200ms
}

sequence "b", length: 1measure {
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "s"
}

show "Show" {
    @9/1
    sequence "b", loop: 2
}
"#;
    close_to(&effect_cue_secs(content, "Show"), &[16.0, 17.0]);
}

#[test]
fn a_body_in_beats_without_a_tempo_map_keeps_its_cues_anchors() {
    // P4: "p" has no tempo map, so "b" (120, its own) is laid out in it in
    // seconds -- @2/1 two seconds in -- but "p" is in beats: played at 60,
    // that cue is four beats on, 4s.
    let content = r#"
sequence "b" {
    tempo {
        bpm: 120
        time_signature: 4/4
    }
    @1/1
    front_wash: static, color: "red", duration: 200ms
    @2/1
    front_wash: static, color: "blue", duration: 200ms
}

sequence "p" {
    @0.000
    back_wash: static, color: "green", duration: 200ms
    sequence "b"
}

show "Show" {
    tempo {
        bpm: 60
        time_signature: 4/4
    }
    @1/1
    sequence "p"
}
"#;
    close_to(&effect_cue_secs(content, "Show"), &[0.0, 4.0]);
}

#[test]
fn numbers_out_of_reach_are_errors_not_panics() {
    let sequence = r#"
sequence "a" {
    @0.000
    front_wash: static, color: "red", duration: 100ms
}
"#;
    for (show, what) in [
        ("@0.000\n    sequence \"a\", loop: 4000000000", "loop count"),
        ("@99999999999.000\n    sequence \"a\"", "time"),
        ("@4294967295/1\n    sequence \"a\"", "bar"),
    ] {
        let content = format!(
            "tempo {{\n    bpm: 120\n    time_signature: 4/4\n}}\n{sequence}\nshow \"S\" {{\n    {show}\n}}\n"
        );
        assert!(parse_light_shows(&content).is_err(), "{what}");
    }
    let long = r#"
show "S" {
    @0.000
    front_wash: static, color: "red", duration: 99999999999999999999s
}
"#;
    assert!(parse_light_shows(long).is_err());
}

#[test]
fn a_layout_passes_over_what_does_not_play() {
    // Mid-edit: a call to a sequence that is not there, a cue whose effect
    // has no duration yet, a sequence that does not parse and nothing
    // plays. The rest is laid out; what fails says why.
    let content = r#"
sequence "a" {
    @0.000
    front_wash: static, color: "red", duration: 500ms
}

sequence "broken" {
    @0.000
    front_wash: static, color: "red"
}

show "S" {
    @0.000
    sequence "a"
    @4.000
    sequence "not_there"
    @8.000
    back_wash: static, color: "red"
    @12.000
    sequence "a", loop: 2
}
"#;
    assert!(parse_light_shows(content).is_err());
    let layouts = crate::lighting::parser::layout_sequence_calls(content, None)
        .unwrap()
        .shows;
    let show = &layouts[0];
    let calls: Vec<(usize, Option<&str>)> = show
        .calls
        .iter()
        .map(|c| (c.cue, c.error.as_deref()))
        .collect();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_eq!(calls[0], (0, None));
    assert_eq!(calls[1].0, 1);
    assert!(
        calls[1].1.is_some_and(|e| e.contains("not_there")),
        "{calls:?}"
    );
    assert_eq!(calls[2], (3, None));
    assert_eq!(show.calls[2].cues.len(), 2);
    assert_eq!(show.cue_errors.len(), 1);
    assert_eq!(show.cue_errors[0].0, 2);
}

#[test]
fn a_layout_of_loop_forever_stops_at_its_cue_cap() {
    let mut body = String::new();
    for i in 0..32 {
        body.push_str(&format!(
            "    @{:.3}\n    front_wash: static, color: \"red\", duration: 100ms\n",
            i as f64 * 0.125
        ));
    }
    let mut show = String::new();
    for i in 0..40 {
        show.push_str(&format!(
            "    @{}.000\n    sequence \"a\", loop: loop\n",
            i * 10
        ));
    }
    let content = format!("sequence \"a\" {{\n{body}}}\nshow \"S\" {{\n{show}}}\n");
    let started = std::time::Instant::now();
    let layouts = crate::lighting::parser::layout_sequence_calls(&content, None)
        .unwrap()
        .shows;
    let calls = &layouts[0].calls;
    assert_eq!(calls.len(), 40);
    for call in calls {
        assert!(call.truncated);
        assert!(call.cues.len() <= crate::lighting::parser::MAX_LAID_OUT_CUES);
        assert!(!call.cues.is_empty());
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn a_pass_in_beats_without_a_length_stays_on_the_beat_grid() {
    // R1: written at 120, its effects last 250ms -- half a beat there -- so
    // a pass is a beat and a half wherever it plays: 1s at 90, though the
    // effects themselves last 250ms there.
    let content = r#"
tempo {
    start: 0.0s
    bpm: 90
    time_signature: 4/4
}

sequence "s" {
    tempo {
        start: 0.0s
        bpm: 120
        time_signature: 4/4
    }
    @1/1
    front_wash: static, color: "red", duration: 250ms
    @1/2
    front_wash: static, color: "blue", duration: 250ms
}

show "Show" {
    @1/1
    sequence "s", loop: 6
}
"#;
    let times = cue_secs(content, "Show");
    assert_eq!(times.len(), 12);
    for pass in 0..6 {
        close_to(
            &times[pass * 2..pass * 2 + 2],
            &[pass as f64, pass as f64 + 2.0 / 3.0],
        );
    }
}

#[test]
fn a_layout_cuts_a_pass_partway_at_its_cue_cap() {
    // One pass of 2,001 cues: the first 2,000 are drawn.
    let mut body = String::new();
    for i in 0..2001 {
        body.push_str(&format!(
            "    @{:.3}\n    front_wash: static, color: \"red\", duration: 100ms\n",
            i as f64 * 0.1
        ));
    }
    let content = format!(
        "sequence \"big\" {{\n{body}}}\nshow \"S\" {{\n    @0.000\n    sequence \"big\"\n}}\n"
    );
    let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
    let call = &layout.shows[0].calls[0];
    assert_eq!(call.cues.len(), crate::lighting::parser::MAX_LAID_OUT_CUES);
    assert!(call.truncated);
    // Played, all of it.
    assert_eq!(parse_light_shows(&content).unwrap()["S"].cues.len(), 2001);
}

#[test]
fn a_layout_caps_a_sequence_played_inside_another() {
    // "verse" plays "pulse" for good: laid out, quickly and in part.
    let content = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "pulse" {
    @1/1
    front_wash: static, color: "red", duration: 1beat
    @1/2
    front_wash: static, color: "blue", duration: 1beat
}

sequence "verse" {
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "pulse", loop: loop
}

show "S" {
    @1/1
    sequence "verse"
}
"#;
    let started = std::time::Instant::now();
    let layout = crate::lighting::parser::layout_sequence_calls(content, None).unwrap();
    let call = &layout.shows[0].calls[0];
    assert!(call.truncated);
    assert!(!call.cues.is_empty());
    assert!(call.cues.len() <= crate::lighting::parser::MAX_LAID_OUT_CUES);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn times_reached_past_a_show_are_errors_and_quick() {
    let tempo = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\n";
    for (body, what) in [
        ("@1/1\n    front_wash: static, color: \"red\", duration: 8000measures", "measures"),
        (
            "@0.000\n    front_wash: static, color: \"red\", up_time: 900000s, hold_time: 900000s, down_time: 900000s",
            "up, hold and down",
        ),
    ] {
        let content = format!(
            "{tempo}sequence \"a\" {{\n    {body}\n}}\nshow \"S\" {{\n    @1/1\n    sequence \"a\", loop: 1000\n}}\n"
        );
        let started = std::time::Instant::now();
        let err = parse_light_shows(&content).err().map(|e| e.to_string());
        assert!(err.as_deref().is_some_and(|e| e.contains("later than a show")), "{what}: {err:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "{what}");
    }
}

#[test]
fn a_cue_dropped_past_a_length_is_said_so() {
    // R9: one pass of "b", a measure at 240 (a second); "s"'s cue 1.5s in
    // falls past it and does not play, on the last pass as on any.
    let content = r#"
tempo {
    start: 0.0s
    bpm: 240
    time_signature: 4/4
}

sequence "s" {
    @0.000
    front_wash: static, color: "red", duration: 200ms
    @1.500
    front_wash: static, color: "blue", duration: 200ms
}

sequence "b", length: 1measure {
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "s"
}

show "Show" {
    @1/1
    sequence "b"
}
"#;
    let shows = parse_light_shows(content).unwrap();
    let show = &shows["Show"];
    assert_eq!(show.cues.len(), 1);
    assert_eq!(show.dropped.len(), 1);
    assert_eq!(show.dropped[0].sequence, "b");
    let warnings = crate::lighting::lint::lint_shows(
        std::slice::from_ref(show),
        &crate::lighting::lint::LintContext::default(),
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.kind == "cue-past-sequence-length"),
        "{:?}",
        warnings.iter().map(|w| w.kind).collect::<Vec<_>>()
    );
    let layout = crate::lighting::parser::layout_sequence_calls(content, None).unwrap();
    assert_eq!(layout.shows[0].calls[0].dropped, 1);
}

#[test]
fn a_layout_says_why_the_player_would_not_load_the_file() {
    // A sequence nothing plays does not parse: the player rejects the file,
    // and the layout says so, drawing the rest.
    let content = r#"
sequence "a" {
    @0.000
    front_wash: static, color: "red", duration: 1s
}

sequence "broken" {
    @0.000
    front_wash: static, color: "nocolor", duration: 1s
}

show "Show" {
    @0.000
    sequence "a"
}
"#;
    assert!(parse_light_shows(content).is_err());
    let layout = crate::lighting::parser::layout_sequence_calls(content, None).unwrap();
    assert!(layout.problem.is_some());
    assert_eq!(layout.shows[0].calls[0].cues.len(), 1);
}

/// The times of a show's cues with effects, in seconds.
fn effect_secs(content: &str, show: &str) -> Vec<f64> {
    parse_light_shows(content).unwrap()[show]
        .cues
        .iter()
        .filter(|c| !c.effects.is_empty())
        .map(|c| c.time.as_secs_f64())
        .collect()
}

#[test]
fn a_cue_without_effects_does_not_lengthen_a_pass_in_beats() {
    // P1: a beat of red, then a `clear()` on beat 3: a pass lasts its
    // effect, as one in seconds does.
    let content = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "s" {
    @1/1
    front_wash: static, color: "red", duration: 1beat
    @1/3
    clear(layer: foreground)
}

show "Show" {
    @1/1
    sequence "s", loop: 3
}
"#;
    close_to(&effect_secs(content, "Show"), &[0.0, 0.5, 1.0]);
    // P1b: 250ms of red, then a `master()` on beat 2.
    let content = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "s" {
    @1/1
    front_wash: static, color: "red", duration: 250ms
    @1/2
    master(layer: midground, intensity: 0.5)
}

show "Show" {
    @1/1
    sequence "s", loop: 3
}
"#;
    close_to(&effect_secs(content, "Show"), &[0.0, 0.25, 0.5]);
}

#[test]
fn a_cue_past_a_length_is_dropped_where_it_plays_however_deep() {
    // P4: at 240, a measure of "b" is a second; "s"'s cue 1.5s in falls
    // past it, played straight from the show or inside "c" (written at
    // 120, where it would not).
    let content = r#"
tempo {
    start: 0.0s
    bpm: 240
    time_signature: 4/4
}

sequence "s" {
    @0.000
    front_wash: static, color: "red", duration: 200ms
    @1.500
    front_wash: static, color: "blue", duration: 200ms
}

sequence "b", length: 1measure {
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "s"
}

sequence "c" {
    tempo {
        start: 0.0s
        bpm: 120
        time_signature: 4/4
    }
    @1/1
    side: static, color: "red", duration: 1beat
    sequence "b"
}

show "Show" {
    @1/1
    sequence "c"
    @3/1
    sequence "b"
}
"#;
    let shows = parse_light_shows(content).unwrap();
    let show = &shows["Show"];
    assert_eq!(show.dropped.len(), 2, "{:?}", show.dropped);
    assert!(show.dropped.iter().all(|d| d.sequence == "b"));
    let calls: Vec<&str> = show.dropped.iter().map(|d| d.call.as_str()).collect();
    assert_eq!(calls, vec!["c", "b"]);
}

#[test]
fn cues_dropped_past_a_length_are_linted_once_a_call_at_show_times() {
    let sequences = r#"
tempo {
    start: 0.0s
    bpm: 240
    time_signature: 4/4
}

sequence "s" {
    @0.000
    front_wash: static, color: "red", duration: 200ms
    @1.500
    front_wash: static, color: "blue", duration: 200ms
}

sequence "b", length: 1measure {
    @1/1
    back_wash: static, color: "green", duration: 1beat
    sequence "s"
}

sequence "c" {
    @1/1
    side: static, color: "red", duration: 1beat
    sequence "b"
}
"#;
    let lint = |show: &crate::lighting::parser::LightShow| {
        crate::lighting::lint::lint_shows(
            std::slice::from_ref(show),
            &crate::lighting::lint::LintContext::default(),
        )
        .into_iter()
        .map(|w| format!("{w:?}"))
        .filter(|w| w.contains("cue-past-sequence-length"))
        .collect::<Vec<_>>()
    };
    // P2: one a pass for good, said once.
    let content =
        format!("{sequences}show \"Show\" {{\n    @1/1\n    sequence \"b\", loop: loop\n}}\n");
    let shows = parse_light_shows(&content).unwrap();
    let show = &shows["Show"];
    assert!(show.dropped.len() > 1000);
    let warnings = lint(show);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains(&format!("{} cues", show.dropped.len())));
    // P3: inside "c", from bar 10 (9s), two passes: each drops its own, at
    // the show's times.
    let content =
        format!("{sequences}show \"Show\" {{\n    @10/1\n    sequence \"c\", loop: 2\n}}\n");
    let shows = parse_light_shows(&content).unwrap();
    let show = &shows["Show"];
    assert_eq!(show.dropped.len(), 2, "{:?}", show.dropped);
    assert!((show.dropped[0].time.as_secs_f64() - 10.5).abs() < 0.01);
    assert!(show.dropped[1].time > show.dropped[0].time);
    let warnings = lint(show);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("2 cues") && warnings[0].contains("10.500"));
}

#[test]
fn a_long_sequence_played_for_good_stops_at_the_end_of_a_show() {
    // P5: 26 bars at 60 (104s) a pass; P6: 100.5s in seconds. Played for
    // good, they stop where a show must end, as they play.
    for content in [
        "tempo {\n    start: 0.0s\n    bpm: 60\n    time_signature: 4/4\n}\nsequence \"long\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n    @26/4\n    front_wash: static, color: \"blue\", duration: 1beat\n}\nshow \"Show\" {\n    @1/1\n    sequence \"long\", loop: loop\n}\n",
        "sequence \"long\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n    @99.500\n    front_wash: static, color: \"blue\", duration: 1s\n}\nshow \"Show\" {\n    @0.000\n    sequence \"long\", loop: loop\n}\n",
    ] {
        let shows = parse_light_shows(content).unwrap();
        let cues = &shows["Show"].cues;
        assert!(cues.len() > 1000, "{}", cues.len());
        assert!(cues
            .iter()
            .all(|c| c.time.as_secs_f64() <= 1.0e6));
    }
    // A count that reaches past it is still an error.
    let content = "sequence \"long\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n    @99.500\n    front_wash: static, color: \"blue\", duration: 1s\n}\nshow \"Show\" {\n    @0.000\n    sequence \"long\", loop: 10000\n}\n";
    assert!(parse_light_shows(content).is_err());
}

#[test]
fn a_sequence_played_for_good_inside_another_keeps_the_others_cues() {
    // 6c: "outer" plays "long" for good, and a back wash on its bar 2:
    // played from bar 30 (116s), the back wash is at 120s, though "long"'s
    // cues come first and reach the end of a show.
    let content = "tempo {\n    start: 0.0s\n    bpm: 60\n    time_signature: 4/4\n}\nsequence \"long\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n    @26/4\n    front_wash: static, color: \"blue\", duration: 1beat\n}\nsequence \"outer\" {\n    @1/1\n    sequence \"long\", loop: loop\n    @2/1\n    back_wash: static, color: \"green\", duration: 1beat\n}\nshow \"Show\" {\n    @30/1\n    sequence \"outer\"\n}\n";
    let back_wash = |content: &str| -> Vec<f64> {
        parse_light_shows(content).unwrap()["Show"]
            .cues
            .iter()
            .filter(|c| {
                c.effects
                    .iter()
                    .any(|e| e.groups.iter().any(|g| g == "back_wash"))
            })
            .map(|c| c.time.as_secs_f64())
            .collect()
    };
    close_to(&back_wash(content), &[120.0]);
    // E1, in seconds: the back wash 1s in, played from 10s.
    let content = "sequence \"inner\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n    @199.000\n    front_wash: static, color: \"blue\", duration: 1s\n}\nsequence \"outer\" {\n    @0.000\n    sequence \"inner\", loop: loop\n    @1.000\n    back_wash: static, color: \"green\", duration: 1s\n}\nshow \"Show\" {\n    @10.000\n    sequence \"outer\"\n}\n";
    close_to(&back_wash(content), &[11.0]);
}

#[test]
fn a_sequence_played_for_good_inside_another_parses_quickly() {
    // 6d, 6e: a long nested loop costs as many cues as it has, not their
    // square (13s, before).
    let content = "tempo {\n    start: 0.0s\n    bpm: 60\n    time_signature: 4/4\n}\nsequence \"long\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n    @2/4\n    front_wash: static, color: \"blue\", duration: 1beat\n}\nsequence \"outer\" {\n    @1/1\n    sequence \"long\", loop: loop\n    @2/1\n    back_wash: static, color: \"green\", duration: 1beat\n}\nshow \"Show\" {\n    @30/1\n    sequence \"outer\"\n}\n";
    let started = std::time::Instant::now();
    let shows = parse_light_shows(content).unwrap();
    assert!(shows["Show"].cues.len() > 10_000);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_pass_is_laid_out_where_it_plays_however_deep() {
    // 6g: "a" plays "b" (a measure), which plays "s" in seconds; from bar 3
    // the song is at 240, where a measure is a second and "s"'s cue 1.5s in
    // falls past it. Played straight or inside "x" (built at 120, where it
    // would not), "a"'s second pass starts a measure on, where "b" ends --
    // as "b" played straight would.
    let tempo = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @3/1 { bpm: 240 }\n    ]\n}\n";
    let defs = "sequence \"s\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 200ms\n    @0.600\n    front_wash: static, color: \"green\", duration: 200ms\n    @1.500\n    front_wash: static, color: \"blue\", duration: 200ms\n}\nsequence \"b\", length: 1measure {\n    @1/1\n    back_wash: static, color: \"green\", duration: 1beat\n    sequence \"s\"\n}\nsequence \"a\" {\n    @1/1\n    sequence \"b\"\n}\nsequence \"x\" {\n    @1/1\n    sequence \"a\", loop: 2\n}\n";
    for show in [
        "show \"Show\" {\n    @3/1\n    sequence \"a\", loop: 2\n}\n",
        "show \"Show\" {\n    @3/1\n    sequence \"x\"\n}\n",
    ] {
        let content = format!("{tempo}{defs}{show}");
        let times = cue_secs(&content, "Show");
        close_to(&times, &[4.0, 4.6, 5.0, 5.6]);
    }
}

#[test]
fn a_sequence_played_for_good_ends_with_the_pass_that_reaches_a_shows_end() {
    let inner = "sequence \"inner\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n    @199.000\n    front_wash: static, color: \"blue\", duration: 1s\n}\n";
    // Z1: its second cue falls past the end: one cue, no more passes.
    let content = format!(
        "{inner}show \"Show\" {{\n    @999900.000\n    sequence \"inner\", loop: loop\n}}\n"
    );
    close_to(&cue_secs(&content, "Show"), &[999_900.0]);
    // Z2: so with no effects.
    let content = "sequence \"inner\" {\n    @0.000\n    clear(layer: foreground)\n    @199.000\n    clear(layer: background)\n}\nshow \"Show\" {\n    @999900.000\n    sequence \"inner\", loop: loop\n}\n";
    close_to(&cue_secs(content, "Show"), &[999_900.0]);
    // E2: two cues a pass, 200s apart, from 10s: the last pass the one whose
    // second cue falls past it, and no pass after.
    let content =
        format!("{inner}show \"Show\" {{\n    @10.000\n    sequence \"inner\", loop: loop\n}}\n");
    let times = cue_secs(&content, "Show");
    assert_eq!(times.len(), 9999);
    close_to(
        &times[times.len() - 3..],
        &[999_610.0, 999_809.0, 999_810.0],
    );
    // E4: "outer" holds "inner" for good; three passes of it are one.
    let content = format!("{inner}sequence \"outer\" {{\n    @0.000\n    sequence \"inner\", loop: loop\n}}\nshow \"Show\" {{\n    @10.000\n    sequence \"outer\", loop: 3\n}}\n");
    let shows = parse_light_shows(&content).unwrap();
    let cues = &shows["Show"].cues;
    assert!(cues
        .iter()
        .all(|c| !c.stop_sequences.contains(&"outer".to_string())));
}

#[test]
fn a_sequence_in_beats_keeps_its_grid_played_inside_another() {
    // v2: "i" lasts a second, written at 120: two beats a pass. Played from
    // bar 5, after the song slows to 60, its passes are two seconds apart --
    // straight from the show, or inside "o", whose own pass is "i"'s.
    let defs = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @3/1 { bpm: 60 }\n    ]\n}\nsequence \"i\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1s\n}\nsequence \"o\" {\n    @1/1\n    sequence \"i\"\n}\n";
    for call in ["sequence \"i\", loop: 3", "sequence \"o\", loop: 3"] {
        let content = format!("{defs}show \"Show\" {{\n    @5/1\n    {call}\n}}\n");
        close_to(&cue_secs(&content, "Show"), &[12.0, 14.0, 16.0]);
    }
}

#[test]
fn a_pass_that_lasts_no_time_is_the_last() {
    // z1: "s"'s blue cue falls past each pass of "b", leaving only its
    // `clear()`: a pass of no time, played once, for good or not.
    for repeat in ["loop", "4"] {
        let content = format!("tempo {{\n    start: 0.0s\n    bpm: 240\n    time_signature: 4/4\n}}\nsequence \"s\" {{\n    @0.000\n    clear(layer: foreground)\n    @1.500\n    front_wash: static, color: \"blue\", duration: 200ms\n}}\nsequence \"b\", length: 1measure {{\n    @1/1\n    back_wash: static, color: \"green\", duration: 1beat\n    sequence \"s\", loop: {repeat}\n}}\nshow \"Show\" {{\n    @1/1\n    sequence \"b\", loop: 4\n}}\n");
        let shows = parse_light_shows(&content).unwrap();
        let show = &shows["Show"];
        close_to(&cue_secs(&content, "Show"), &[0.0, 1.0, 2.0, 3.0]);
        assert_eq!(show.dropped.len(), 4);
        let clears: usize = show.cues.iter().map(|c| c.layer_commands.len()).sum();
        assert_eq!(clears, 4);
    }
}

#[test]
fn a_layout_counts_what_does_not_play_against_its_cap() {
    // p2, p3: passes of "in" dropped past each measure of "out", thousands
    // of them: laid out to the cap, and quickly.
    let head = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\nsequence \"in\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n}\n";
    for body in [
        "sequence \"out\", length: 1measure {\n    @1/1\n    sequence \"in\", loop: loop\n}\nshow \"Show\" {\n    @1/1\n    sequence \"out\", loop: 200\n}\n",
        "sequence \"out\", length: 1measure {\n    @1/1\n    sequence \"in\", loop: 1000\n}\nsequence \"out2\", length: 2measures {\n    @1/1\n    sequence \"out\", loop: 1000\n}\nshow \"Show\" {\n    @1/1\n    sequence \"out2\", loop: 10\n}\n",
    ] {
        let content = format!("{head}{body}");
        let started = std::time::Instant::now();
        let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        let call = &layout.shows[0].calls[0];
        assert!(call.cues.len() <= crate::lighting::parser::MAX_LAID_OUT_CUES);
    }
}

#[test]
fn a_call_cut_short_ends_where_what_it_played_does() {
    // p1: sequences seven deep, more cues than a layout draws: the call cut
    // short still lasts as far as what was laid out.
    let mut content = String::from("tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\nsequence \"l0\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n    @1/3\n    back_wash: static, color: \"blue\", duration: 1beat\n}\n");
    for level in 1..=6 {
        content.push_str(&format!(
            "sequence \"l{level}\" {{\n    @1/1\n    side_wash: static, color: \"red\", duration: 1beat\n    sequence \"l{}\", loop: 4\n}}\n",
            level - 1
        ));
    }
    content.push_str("show \"Show\" {\n    @1/1\n    sequence \"l6\"\n}\n");
    let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
    let call = &layout.shows[0].calls[0];
    assert!(call.truncated);
    let last = call.cues.iter().map(|c| c.time).max().unwrap();
    assert!(call.end >= last && call.end > call.start, "{:?}", call.end);
}

#[test]
fn a_sequence_without_effects_inside_one_with_some_does_not_lengthen_it() {
    // s2 has no effect and lasts 1.5s; "s1"'s pass is its own effect's
    // second, as when it was laid out with what it plays.
    let content = "sequence \"s2\", length: 1.5s {\n    @1.000\n    clear(layer: foreground)\n}\nsequence \"s1\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n    sequence \"s2\"\n}\nshow \"Show\" {\n    @0.000\n    sequence \"s1\", loop: 2\n}\n";
    let shows = parse_light_shows(content).unwrap();
    let starts: Vec<f64> = shows["Show"]
        .cues
        .iter()
        .filter(|c| !c.effects.is_empty())
        .map(|c| c.time.as_secs_f64())
        .collect();
    close_to(&starts, &[0.0, 1.0]);
}

/// The times of the cues with effects of `seq` played three times from bar
/// 2: straight, wrapped once and wrapped twice (`@1/1 sequence "…"`), with
/// `defs` and the file's `tempo`.
fn wrapped_three_ways(tempo: &str, defs: &str, seq: &str) -> [Vec<f64>; 3] {
    let wraps = format!(
        "sequence \"w1\" {{\n    @1/1\n    sequence \"{seq}\"\n}}\nsequence \"w2\" {{\n    @1/1\n    sequence \"w1\"\n}}\n"
    );
    ["", "w1", "w2"].map(|wrap| {
        let call = if wrap.is_empty() { seq } else { wrap };
        let content = format!(
            "{tempo}{defs}{wraps}show \"Show\" {{\n    @2/1\n    sequence \"{call}\", loop: 3\n}}\n"
        );
        effect_cue_secs(&content, "Show")
    })
}

fn assert_wraps_alike(tempo: &str, defs: &str, seq: &str) -> Vec<f64> {
    let [direct, once, twice] = wrapped_three_ways(tempo, defs, seq);
    close_to(&once, &direct);
    close_to(&twice, &direct);
    direct
}

const AT_120: &str = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\n";

#[test]
fn a_sequence_with_its_own_tempo_keeps_its_timing_wrapped() {
    // "i" is written at 60: its second-long effect is a beat, half a second
    // at the song's 120.
    let direct = assert_wraps_alike(
        AT_120,
        "sequence \"i\" {\n    tempo {\n        start: 0.0s\n        bpm: 60\n        time_signature: 4/4\n    }\n    @1/1\n    front_wash: static, color: \"red\", duration: 1s\n}\n",
        "i",
    );
    close_to(&direct, &[2.0, 2.5, 3.0]);
    // In 3/4, a measure long: three beats, 1.5s.
    let direct = assert_wraps_alike(
        AT_120,
        "sequence \"i\" {\n    tempo {\n        start: 0.0s\n        bpm: 120\n        time_signature: 3/4\n    }\n    @1/1\n    front_wash: static, color: \"red\", duration: 1measure\n}\n",
        "i",
    );
    close_to(&direct, &[2.0, 3.5, 5.0]);
}

#[test]
fn a_sequence_with_a_length_keeps_it_wrapped() {
    // In seconds, 2s a pass; in beats, a measure (2s at 120).
    for length in ["2s", "1measure"] {
        let defs = format!(
            "sequence \"s\", length: {length} {{\n    @0.000\n    front_wash: static, color: \"red\", duration: 200ms\n}}\n"
        );
        let direct = assert_wraps_alike(AT_120, &defs, "s");
        close_to(&direct, &[2.0, 4.0, 6.0]);
    }
}

#[test]
fn a_sequence_with_its_own_tempo_and_none_around_keeps_its_timing_wrapped() {
    // No file tempo, the show's at 120: "i", written at 60, lasts two of its
    // beats -- two seconds -- however it is wrapped.
    let tempo = "";
    let defs = "sequence \"i\" {\n    tempo {\n        start: 0.0s\n        bpm: 60\n        time_signature: 4/4\n    }\n    @1/1\n    front_wash: static, color: \"red\", duration: 2beats\n}\nsequence \"o\" {\n    @0.000\n    sequence \"i\"\n}\nsequence \"o120\" {\n    tempo {\n        start: 0.0s\n        bpm: 120\n        time_signature: 4/4\n    }\n    @1/1\n    sequence \"i\"\n}\n";
    let show = |call: &str| {
        format!(
            "{tempo}{defs}show \"Show\" {{\n    tempo {{\n        start: 0.0s\n        bpm: 120\n        time_signature: 4/4\n    }}\n    @0.000\n    sequence \"{call}\", loop: 3\n}}\n"
        )
    };
    let direct = effect_cue_secs(&show("i"), "Show");
    close_to(&effect_cue_secs(&show("o"), "Show"), &direct);
    close_to(&effect_cue_secs(&show("o120"), "Show"), &direct);
}

#[test]
fn a_cue_that_only_plays_a_sequence_does_not_count_against_a_layouts_cap() {
    let defs = "sequence \"i\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n}\nsequence \"w\" {\n    @1/1\n    sequence \"i\"\n}\nsequence \"o2\" {\n    @1/1\n    sequence \"i\", loop: 2000\n}\n";
    for (call, n, truncated) in [
        ("sequence \"w\", loop: 1500", 1500, false),
        ("sequence \"o2\"", 2000, false),
    ] {
        let content = format!("{AT_120}{defs}show \"Show\" {{\n    @1/1\n    {call}\n}}\n");
        let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
        let laid = &layout.shows[0].calls[0];
        assert_eq!(laid.cues.len(), n, "{call}");
        assert_eq!(laid.truncated, truncated, "{call}");
    }
}

#[test]
fn a_call_cut_short_ends_where_what_it_laid_out_does_with_no_effects() {
    // "n" has no effects, inside "p", which has: cut short, the call still
    // ends where "n"'s cues laid out do.
    let content = format!("{AT_120}sequence \"n\" {{\n    @1/1\n    clear(layer: foreground)\n    @1/2\n    clear(layer: background)\n}}\nsequence \"p\" {{\n    @1/1\n    sequence \"n\", loop: 3000\n    @2/1\n    front_wash: static, color: \"red\", duration: 1beat\n}}\nshow \"Show\" {{\n    @2/1\n    sequence \"p\"\n}}\n");
    let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
    let call = &layout.shows[0].calls[0];
    assert!(call.truncated);
    let last = call.cues.iter().map(|c| c.time).max().unwrap();
    assert!(call.end >= last && call.end > call.start, "{:?}", call.end);
}

#[test]
fn a_length_in_seconds_lasts_its_seconds_where_it_plays() {
    // F2: "s" is in beats, its `length:` in seconds: played from bar 4,
    // after the song speeds to 240, its passes are 0.7s apart -- straight
    // or wrapped -- not 0.7s at the 120 it was written at.
    let defs = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @3/1 { bpm: 240 }\n    ]\n}\nsequence \"s\", length: 0.7s {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n}\nsequence \"w1\" {\n    @1/1\n    sequence \"s\"\n}\nsequence \"w2\" {\n    @1/1\n    sequence \"w1\"\n}\n";
    for call in ["s", "w1", "w2"] {
        let content =
            format!("{defs}show \"Show\" {{\n    @4/1\n    sequence \"{call}\", loop: 3\n}}\n");
        close_to(&effect_cue_secs(&content, "Show"), &[5.0, 5.7, 6.4]);
    }
}

#[test]
fn a_fan_out_of_empty_sequences_lays_out_quickly() {
    // F1: six levels of ten calls each, to an empty sequence: a million
    // calls that lay out nothing, bounded by the cap as cues are.
    let mut content = String::from(AT_120);
    content.push_str("sequence \"w0\" {\n}\n");
    for level in 1..=6 {
        content.push_str(&format!("sequence \"w{level}\" {{\n"));
        for beat in 1..=10 {
            content.push_str(&format!("    @1/{beat}\n    sequence \"w{}\"\n", level - 1));
        }
        content.push_str("}\n");
    }
    content.push_str("show \"Show\" {\n    @1/1\n    sequence \"w6\"\n}\n");
    let started = std::time::Instant::now();
    let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    let call = &layout.shows[0].calls[0];
    assert!(call.cues.is_empty() && !call.truncated);
}

/// Sequences "w1" to "w{depth}", each ten calls (on beats 1 to 10 of bar 1)
/// to the one below; "w0" is `w0`.
fn fan_out(depth: usize, w0: &str) -> String {
    let mut content = String::from(AT_120);
    content.push_str(w0);
    for level in 1..=depth {
        content.push_str(&format!("sequence \"w{level}\" {{\n"));
        for beat in 1..=10 {
            content.push_str(&format!("    @1/{beat}\n    sequence \"w{}\"\n", level - 1));
        }
        content.push_str("}\n");
    }
    content.push_str(&format!(
        "show \"Show\" {{\n    @1/1\n    sequence \"w{depth}\"\n}}\n"
    ));
    content
}

#[test]
fn a_fan_out_of_hollow_sequences_parses_quickly() {
    // Played, a fan-out of calls to sequences that play nothing costs no
    // more than its calls: seven levels of ten (ten million calls).
    for w0 in [
        "sequence \"w0\" {\n}\n",
        "sequence \"w0\" {\n    @0.000\n    sequence \"z\"\n}\nsequence \"z\" {\n}\n",
    ] {
        let content = fan_out(7, w0);
        let started = std::time::Instant::now();
        let shows = parse_light_shows(&content).unwrap();
        assert!(shows["Show"].cues.is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }
}

#[test]
fn a_hollow_sequence_reaches_as_far_as_its_calls() {
    // "w1" calls the empty "w0" on beats 1 and 3: a pass of two beats.
    // Three passes inside "w2", twice: "w2"'s passes are six beats.
    let content = format!("{AT_120}sequence \"w0\" {{\n}}\nsequence \"w1\" {{\n    @1/1\n    sequence \"w0\"\n    @1/3\n    sequence \"w0\"\n}}\nsequence \"w2\" {{\n    @1/1\n    sequence \"w1\", loop: 3\n}}\nshow \"Show\" {{\n    @1/1\n    sequence \"w2\", loop: 2\n}}\n");
    let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
    let call = &layout.shows[0].calls[0];
    let ends: Vec<f64> = call.pass_ends.iter().map(|d| d.as_secs_f64()).collect();
    close_to(&ends, &[3.0, 6.0]);
}

#[test]
fn calls_that_lay_out_nothing_do_not_eat_a_layouts_cap() {
    let defs = "sequence \"todo\" {\n}\nsequence \"b\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n    sequence \"todo\"\n    sequence \"todo\"\n    sequence \"todo\"\n}\nsequence \"c\" {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n    @1/2\n    sequence \"todo\"\n}\n";
    for (call, n) in [
        ("sequence \"b\", loop: 600", 600),
        ("sequence \"c\", loop: 2000", 2000),
    ] {
        let content = format!("{AT_120}{defs}show \"Show\" {{\n    @1/1\n    {call}\n}}\n");
        let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
        let laid = &layout.shows[0].calls[0];
        assert_eq!(laid.cues.len(), n, "{call}");
        assert!(!laid.truncated, "{call}");
        assert_eq!(laid.pass_ends.len(), n, "{call}");
    }
}

#[test]
fn a_length_in_seconds_does_not_drift() {
    // At 160, a second a pass for 200 passes, through a change to 90: each
    // pass starts exactly n seconds on, ends where the next starts, and so
    // when wrapped.
    let defs = "tempo {\n    start: 0.0s\n    bpm: 160\n    time_signature: 4/4\n    changes: [\n        @40/1 { bpm: 90 }\n    ]\n}\nsequence \"s\", length: 1s {\n    @1/1\n    front_wash: static, color: \"red\", duration: 1beat\n}\nsequence \"w\" {\n    @1/1\n    sequence \"s\"\n}\n";
    let start = Duration::from_millis(1500); // bar 2 at 160
    let mut direct = Vec::new();
    for call in ["s", "w"] {
        let content =
            format!("{defs}show \"Show\" {{\n    @2/1\n    sequence \"{call}\", loop: 200\n}}\n");
        let shows = parse_light_shows(&content).unwrap();
        let times: Vec<Duration> = shows["Show"]
            .cues
            .iter()
            .filter(|c| !c.effects.is_empty())
            .map(|c| c.time)
            .collect();
        assert_eq!(times.len(), 200);
        for (n, time) in times.iter().enumerate() {
            assert_eq!(
                *time,
                start + Duration::from_secs(n as u64),
                "{call} pass {n}"
            );
        }
        if call == "s" {
            let layout = crate::lighting::parser::layout_sequence_calls(&content, None).unwrap();
            let ends = &layout.shows[0].calls[0].pass_ends;
            for (n, end) in ends.iter().enumerate() {
                assert_eq!(*end, start + Duration::from_secs(n as u64 + 1), "end {n}");
            }
            direct = times;
        } else {
            assert_eq!(times, direct);
        }
    }
}
