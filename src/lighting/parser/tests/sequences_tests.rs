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
    release(layer: background, time: 2s)
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

    // Second cue should have release command with time
    let second_cue = &show.cues[1];
    assert_eq!(second_cue.layer_commands.len(), 1);
    let release_cmd = &second_cue.layer_commands[0];
    assert_eq!(
        release_cmd.command_type,
        crate::lighting::parser::LayerCommandType::Release
    );
    assert_eq!(
        release_cmd.layer,
        Some(crate::lighting::effects::EffectLayer::Background)
    );
    assert!(release_cmd.fade_time.is_some());

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
    // This verifies the fix for playback_measures_to_duration handling fractional measures
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
