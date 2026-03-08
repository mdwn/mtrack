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
use crate::lighting::parser::types::LayerCommandType;
use crate::lighting::parser::*;

// ===================================================================
// 1. Sequence commands: reset_measures_command, layer_command,
//    stop_sequence_command parsing in sequence cues
// ===================================================================

#[test]
fn test_sequence_cue_with_reset_measures_command() {
    // Test that reset_measures works inside a sequence cue.
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "with_reset" {
    @1/1
    all_wash: static, color: "red"

    offset 4 measures

    @1/1
    all_wash: static, color: "green"

    reset_measures

    @1/1
    all_wash: static, color: "blue"
}

show "Test" {
    @0.0
    sequence "with_reset"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse sequence with reset_measures: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 measure = 2.0s
    // Sequence internal cue times:
    //   @1/1 = 0.0s (no offset yet)
    //   offset 4 => offset_secs = 8.0
    //   @1/1 + 8.0 = 8.0s
    //   reset_measures => offset_secs = 0.0
    //   @1/1 + 0.0 = 0.0s
    // Internal times: [0.0, 8.0, 0.0]
    // When expanded from show at @0.0, cues are sorted by time: [0.0, 0.0, 8.0]
    assert!(show.cues.len() >= 3, "Should have at least 3 cues");

    // Collect all cue times
    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();

    // Should have cues at 0.0 (x2) and 8.0
    let count_at_0 = times.iter().filter(|&&t| t.abs() < 0.001).count();
    let count_at_8 = times.iter().filter(|&&t| (t - 8.0).abs() < 0.001).count();

    assert_eq!(
        count_at_0, 2,
        "Should have 2 cues at 0.0s, got times: {:?}",
        times
    );
    assert_eq!(
        count_at_8, 1,
        "Should have 1 cue at 8.0s, got times: {:?}",
        times
    );
}

#[test]
fn test_sequence_cue_with_layer_command() {
    // Test that layer commands inside a sequence cue are parsed correctly.
    let content = r#"
sequence "with_layers" {
    @0.000
    all_wash: static, color: "red", layer: foreground

    @2.000
    clear(layer: foreground)

    @3.000
    freeze(layer: background)
}

show "Test" {
    @0.000
    sequence "with_layers"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse sequence with layer commands: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    assert_eq!(show.cues.len(), 3, "Should have 3 cues");

    // First cue: effect at 0.0s
    assert_eq!(show.cues[0].effects.len(), 1);
    assert_eq!(show.cues[0].layer_commands.len(), 0);

    // Second cue: clear command at 2.0s
    assert_eq!(show.cues[1].effects.len(), 0);
    assert_eq!(show.cues[1].layer_commands.len(), 1);
    assert_eq!(
        show.cues[1].layer_commands[0].command_type,
        LayerCommandType::Clear
    );

    // Third cue: freeze command at 3.0s
    assert_eq!(show.cues[2].effects.len(), 0);
    assert_eq!(show.cues[2].layer_commands.len(), 1);
    assert_eq!(
        show.cues[2].layer_commands[0].command_type,
        LayerCommandType::Freeze
    );
}

#[test]
fn test_sequence_cue_with_stop_sequence_command() {
    // Test that stop sequence commands inside a sequence cue work.
    let content = r#"
sequence "looper" {
    @0.000
    all_wash: static, color: "red"
}

sequence "main_seq" {
    @0.000
    sequence "looper", loop: loop

    @5.000
    stop sequence "looper"
    all_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "main_seq"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse sequence with stop command: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find the cue at 5.0s with stop_sequences
    let stop_cue = show.cues.iter().find(|c| {
        (c.time.as_secs_f64() - 5.0).abs() < 0.01
            && c.stop_sequences.contains(&"looper".to_string())
    });
    assert!(
        stop_cue.is_some(),
        "Should have a cue at 5.0s that stops 'looper'"
    );
}

// ===================================================================
// 2. Measure-based timing without tempo: Error "Measure-based timing
//    requires a tempo section"
// ===================================================================

#[test]
fn test_measure_timing_without_tempo_in_show() {
    // Using measure-based timing (e.g., @1/1) without any tempo section should fail.
    let content = r#"
show "No Tempo" {
    @1/1
    front_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Should fail without tempo section");

    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Measure-based timing requires a tempo section"),
        "Error should mention tempo requirement: {}",
        error
    );
}

#[test]
fn test_measure_timing_without_tempo_in_sequence() {
    // Using measure-based timing in a sequence without tempo should fail.
    let content = r#"
sequence "no_tempo_seq" {
    @1/1
    front_wash: static, color: "red"
}

show "Test" {
    @0.0
    sequence "no_tempo_seq"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_err(),
        "Should fail with measure-based timing and no tempo"
    );

    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Measure-based timing requires a tempo section"),
        "Error should mention tempo requirement: {}",
        error
    );
}

// ===================================================================
// 3. Circular sequence references: Error "Circular sequence reference
//    detected"
// ===================================================================

#[test]
fn test_circular_reference_three_sequences() {
    // A -> B -> C -> A should be detected as circular.
    let content = r#"
sequence "A" {
    @0.000
    front_wash: static, color: "red"
    @1.000
    sequence "B"
}

sequence "B" {
    @0.000
    sequence "C"
}

sequence "C" {
    @0.000
    sequence "A"
}

show "Test" {
    @0.000
    sequence "A"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_err(),
        "Should detect circular reference A->B->C->A"
    );

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Circular sequence reference"),
        "Error should mention circular reference: {}",
        error
    );
}

#[test]
fn test_direct_self_reference() {
    // A sequence referencing itself directly.
    let content = r#"
sequence "self_ref" {
    @0.000
    front_wash: static, color: "red"
    @1.000
    sequence "self_ref"
}

show "Test" {
    @0.000
    sequence "self_ref"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Should detect self-reference");

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Circular sequence reference"),
        "Error should mention circular reference: {}",
        error
    );
}

// ===================================================================
// 5. parse_sequence_loop_param(): Error paths for invalid loop count
// ===================================================================

#[test]
fn test_sequence_loop_param_zero() {
    // Loop count of 0 should error with "Loop count must be at least 1".
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "seq", loop: 0
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Loop count 0 should fail");

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Loop count must be at least 1"),
        "Error should say loop count must be at least 1: {}",
        error
    );
}

#[test]
fn test_sequence_loop_param_valid_count() {
    // Loop count of 2 should produce 2 iterations.
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "red"

    @1.000
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "seq", loop: 2
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Loop count 2 should succeed: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // 2 cues per iteration * 2 iterations = 4 cues
    assert_eq!(show.cues.len(), 4, "Should have 4 cues for loop: 2");
}

// ===================================================================
// 7. expand_unexpanded_sequence_cue(): Edge cases - stop_sequences
//    in loops, empty expanded_cues with effects
// ===================================================================

#[test]
fn test_stop_sequences_in_looped_sequence() {
    // When a looped sequence cue also has stop_sequences, they should
    // be merged into the first expanded cue.
    let content = r#"
sequence "bg" {
    @0.000
    back_wash: static, color: "blue"
}

sequence "fg" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "bg", loop: loop

    @5.000
    stop sequence "bg"
    sequence "fg"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse stop with sequence ref: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find the cue at 5.0s
    let cue_at_5 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 5.0).abs() < 0.01);
    assert!(cue_at_5.is_some(), "Should have a cue at 5.0s");
    let cue = cue_at_5.unwrap();

    // The stop_sequences should include "bg"
    assert!(
        cue.stop_sequences.contains(&"bg".to_string()),
        "Cue at 5.0s should stop 'bg', got: {:?}",
        cue.stop_sequences
    );
}

#[test]
fn test_effects_with_sequence_reference_merged() {
    // When a cue has both effects and a sequence reference,
    // the effects should be merged into the first expanded cue.
    let content = r#"
sequence "inner" {
    @0.000
    front_wash: static, color: "blue"

    @1.000
    front_wash: static, color: "green"
}

show "Test" {
    @2.000
    back_wash: static, color: "red"
    sequence "inner"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Should parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // The cue at 2.0 should have both the red effect and the first inner cue's blue effect
    let cue_at_2 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 2.0).abs() < 0.01);
    assert!(cue_at_2.is_some(), "Should have a cue at 2.0s");
    let cue = cue_at_2.unwrap();
    assert!(
        cue.effects.len() >= 2,
        "Cue at 2.0 should have at least 2 effects (base + sequence first cue), got {}",
        cue.effects.len()
    );
}

// ===================================================================
// 8. parse_and_expand_inline_loop(): Edge cases - empty loop body,
//    perpetual effects duration calculation
// ===================================================================

#[test]
fn test_inline_loop_with_perpetual_effects_multi_cue() {
    // When all effects in a loop are perpetual (no duration), the loop
    // duration should be calculated from the relative time between
    // first and last cue. Two perpetual-effect cues 1.0s apart,
    // repeats 3 => cues at 0, 1, 1, 2, 2, 3.
    let content = r#"show "Perpetual Loop" {
    tempo { bpm: 120 }
    @0.0
    loop {
        @0.0
        effect: static, color: "red"
        @1.0
        effect: static, color: "blue"
    }, repeats: 3
}"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse perpetual effects in loop: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = &shows["Perpetual Loop"];

    // Loop duration = 1.0s (time from first cue 0.0 to last cue 1.0)
    // 3 repeats => 6 cues at: 0,1, 1,2, 2,3
    assert_eq!(show.cues.len(), 6, "Expected 6 cues for perpetual loop x3");

    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();
    let expected = [0.0, 1.0, 1.0, 2.0, 2.0, 3.0];
    for (i, &exp) in expected.iter().enumerate() {
        assert!(
            (times[i] - exp).abs() < 0.01,
            "Cue {} should be at {}s, got {}s",
            i,
            exp,
            times[i]
        );
    }
}

#[test]
fn test_inline_loop_single_perpetual_cue() {
    // A loop with a single perpetual effect should produce duration ZERO,
    // so all repetitions collapse to the same time.
    let content = r#"show "Single Perpetual" {
    tempo { bpm: 120 }
    @0.0
    loop {
        @0.0
        effect: static, color: "red"
    }, repeats: 3
}"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse single perpetual loop: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = &shows["Single Perpetual"];

    // Duration is 0 for single-cue perpetual loop, so all 3 iterations
    // should produce cues at 0.0.
    assert_eq!(show.cues.len(), 3, "Expected 3 cues");

    for cue in &show.cues {
        assert!(
            cue.time.as_secs_f64().abs() < 0.01,
            "All cues should be at 0.0s"
        );
    }
}

// ===================================================================
// 9. parse_cue_definition(): Layer commands with sequence references
// ===================================================================

#[test]
fn test_cue_with_layer_command_and_sequence_reference() {
    // A cue that has both a layer command and a sequence reference should
    // have both parsed correctly.
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    front_wash: static, color: "red", layer: background

    @5.000
    clear(layer: foreground)
    sequence "seq"

    @10.000
    release(layer: background, time: 1s)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse layer + sequence in same cue: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find cue at 5.0s - should have both the layer command and the sequence effect
    let cue_at_5 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 5.0).abs() < 0.01);
    assert!(cue_at_5.is_some(), "Should have a cue at 5.0s");
    let cue = cue_at_5.unwrap();

    // Should have the clear layer command
    assert!(
        cue.layer_commands
            .iter()
            .any(|lc| lc.command_type == LayerCommandType::Clear),
        "Cue at 5.0s should have clear layer command"
    );

    // Should have at least one effect from the sequence
    assert!(
        !cue.effects.is_empty(),
        "Cue at 5.0s should have effects from sequence"
    );

    // Find cue at 10.0s - should have release command
    let cue_at_10 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 10.0).abs() < 0.01);
    assert!(cue_at_10.is_some(), "Should have a cue at 10.0s");
    let cue = cue_at_10.unwrap();
    assert!(
        cue.layer_commands
            .iter()
            .any(|lc| lc.command_type == LayerCommandType::Release),
        "Cue at 10.0s should have release layer command"
    );
    assert!(
        cue.layer_commands[0].fade_time.is_some(),
        "Release command should have fade_time"
    );
}

#[test]
fn test_cue_with_multiple_layer_commands_and_effects() {
    // A cue that has multiple layer commands alongside effects and a sequence ref.
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    front_wash: static, color: "red", layer: background

    @5.000
    clear(layer: foreground)
    master(layer: midground, intensity: 75%)
    back_wash: static, color: "green"
    sequence "seq"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse multiple layer commands + effects + sequence: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find cue at 5.0s
    let cue_at_5 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 5.0).abs() < 0.01);
    assert!(cue_at_5.is_some(), "Should have a cue at 5.0s");
    let cue = cue_at_5.unwrap();

    // Should have 2 layer commands (clear + master)
    assert!(
        cue.layer_commands.len() >= 2,
        "Cue should have at least 2 layer commands, got {}",
        cue.layer_commands.len()
    );

    // Should have at least 2 effects (green + blue from sequence)
    assert!(
        cue.effects.len() >= 2,
        "Cue should have at least 2 effects, got {}",
        cue.effects.len()
    );
}

// ===================================================================
// Additional edge cases: sequence with stop_sequences and effects
// when expanded_cues is empty
// ===================================================================

#[test]
fn test_stop_sequences_without_sequence_ref_effects() {
    // A cue that has stop_sequences but no sequence reference should
    // still produce a cue with the stop_sequences.
    let content = r#"
sequence "bg" {
    @0.000
    back_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "bg", loop: loop

    @5.000
    stop sequence "bg"

    @10.000
    front_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Should parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find the standalone stop cue at 5.0s
    let stop_cue = show.cues.iter().find(|c| {
        (c.time.as_secs_f64() - 5.0).abs() < 0.01 && c.stop_sequences.contains(&"bg".to_string())
    });
    assert!(stop_cue.is_some(), "Should have stop cue at 5.0s");
}

// ===================================================================
// Sequence cue with layer command and stop sequence combined
// ===================================================================

#[test]
fn test_sequence_cue_with_layer_and_stop_combined() {
    // A sequence cue with both a layer command and a stop sequence.
    let content = r#"
sequence "inner" {
    @0.000
    front_wash: static, color: "red"
}

sequence "outer" {
    @0.000
    sequence "inner", loop: loop

    @5.000
    stop sequence "inner"
    clear(layer: foreground)
    back_wash: static, color: "green"
}

show "Test" {
    @0.000
    sequence "outer"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse combined layer + stop in sequence: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find the cue at 5.0s with the stop and layer command
    let combined_cue = show.cues.iter().find(|c| {
        (c.time.as_secs_f64() - 5.0).abs() < 0.01 && c.stop_sequences.contains(&"inner".to_string())
    });
    assert!(
        combined_cue.is_some(),
        "Should have a combined stop+layer cue at 5.0s"
    );
    let cue = combined_cue.unwrap();
    assert!(
        cue.layer_commands
            .iter()
            .any(|lc| lc.command_type == LayerCommandType::Clear),
        "Combined cue should have clear layer command"
    );
}

// ===================================================================
// Inline loop with no base content (only loop, no base effects)
// ===================================================================

#[test]
fn test_inline_loop_only_no_base_effects() {
    // A cue with only an inline loop (no base effects) should produce
    // just the loop cues.
    let content = r#"show "Loop Only" {
    tempo { bpm: 120 }
    @0.0
    loop {
        @0.0
        effect: static, color: "red", duration: 1s
        @1.0
        effect: static, color: "blue", duration: 1s
    }, repeats: 2
}"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse loop-only cue: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = &shows["Loop Only"];

    // 2 cues per iteration * 2 iterations = 4 cues
    assert_eq!(show.cues.len(), 4, "Expected 4 cues from loop x2");

    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();
    let expected = [0.0, 1.0, 2.0, 3.0];
    for (i, &exp) in expected.iter().enumerate() {
        assert!(
            (times[i] - exp).abs() < 0.01,
            "Cue {} should be at {}s, got {}s",
            i,
            exp,
            times[i]
        );
    }
}

// ===================================================================
// Sequence reference with 'once' loop parameter
// ===================================================================

#[test]
fn test_sequence_loop_once_explicit() {
    // Explicit loop: once should behave the same as no loop parameter.
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "red"

    @1.000
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "seq", loop: once
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse loop: once: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Should have exactly 2 cues (1 iteration)
    assert_eq!(show.cues.len(), 2, "loop: once should produce 2 cues");
}

// ===================================================================
// Forward sequence reference (sequence defined after reference)
// ===================================================================

#[test]
fn test_forward_sequence_reference() {
    // Sequence B references A, but A is defined after B. This tests
    // the two-pass expansion logic for forward references.
    let content = r#"
sequence "B" {
    @0.000
    front_wash: static, color: "green"
    @1.000
    sequence "A"
}

sequence "A" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "B"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Forward reference should work with two-pass parsing: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // B has 2 cues: green at 0, then A (red at 1)
    assert!(
        show.cues.len() >= 2,
        "Should have at least 2 cues: {}",
        show.cues.len()
    );
}

// ===================================================================
// Sequence with looped stop_sequences across iterations
// ===================================================================

#[test]
fn test_looped_sequence_stop_sequences_per_iteration() {
    // When a sequence is looped, each iteration (after the first) should
    // have a stop_sequences entry to prevent effect accumulation.
    let content = r#"
sequence "looped" {
    @0.000
    front_wash: static, color: "red"

    @1.000
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "looped", loop: 3
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Should parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    assert_eq!(show.cues.len(), 6, "Should have 6 cues (2 * 3 iterations)");

    // First iteration, first cue: should have start_sequences, no stop
    assert!(
        show.cues[0].start_sequences.contains(&"looped".to_string()),
        "First cue should mark sequence start"
    );
    assert!(
        show.cues[0].stop_sequences.is_empty(),
        "First iteration should not stop anything"
    );

    // Second iteration, first cue: should have stop_sequences
    assert!(
        show.cues[2].stop_sequences.contains(&"looped".to_string()),
        "Second iteration's first cue should stop previous iteration"
    );

    // Third iteration, first cue: should have stop_sequences
    assert!(
        show.cues[4].stop_sequences.contains(&"looped".to_string()),
        "Third iteration's first cue should stop previous iteration"
    );
}

// ===================================================================
// Sequence with own tempo used in show with different tempo
// ===================================================================

#[test]
fn test_sequence_with_own_tempo_rescaling() {
    // A sequence at 60 BPM used in a show at 120 BPM should have its
    // internal timing rescaled.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "slow_seq" {
    tempo {
        bpm: 60
        time_signature: 4/4
    }

    @1/1
    front_wash: static, color: "red"

    @2/1
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "slow_seq"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse sequence with own tempo: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    assert_eq!(show.cues.len(), 2);

    // At 60 BPM, @1/1 = 0.0s, @2/1 = 4.0s (1 measure = 4 beats * 1s/beat)
    // Rescaled to 120 BPM: 4 beats => 4 * 0.5s = 2.0s
    // First cue at 0.0, second cue at 2.0
    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.01,
        "First cue should be at 0.0s"
    );
    assert!(
        (show.cues[1].time.as_secs_f64() - 2.0).abs() < 0.01,
        "Second cue should be rescaled to 2.0s at 120 BPM, got {:?}",
        show.cues[1].time
    );
}

// ===================================================================
// Multiple inline loops in a single cue
// ===================================================================

#[test]
fn test_multiple_inline_loops_in_single_show_cue() {
    // A show cue with two inline loops at different timing offsets.
    let content = r#"show "Multi Loop" {
    tempo { bpm: 120 }
    @0.0
    loop {
        @0.0
        effect: static, color: "red", duration: 1s
    }, repeats: 2
    loop {
        @0.0
        effect: static, color: "blue", duration: 0.5s
    }, repeats: 2
}"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse multiple inline loops: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = &shows["Multi Loop"];

    // First loop: 1s duration, 2 repeats => cues at 0.0, 1.0
    // Second loop: 0.5s duration, 2 repeats => cues at 0.0, 0.5
    // Cues at 0.0 should be merged
    // Total unique times: 0.0, 0.5, 1.0
    assert!(
        show.cues.len() >= 3,
        "Should have at least 3 cues, got {}",
        show.cues.len()
    );
}

// ===================================================================
// Sequence reference from show cue with stop_sequences
// ===================================================================

#[test]
fn test_show_cue_stop_and_start_sequence_at_same_time() {
    // Stop one sequence and start another at the same time.
    let content = r#"
sequence "old" {
    @0.000
    front_wash: static, color: "red"
}

sequence "new" {
    @0.000
    front_wash: static, color: "blue"
}

show "Test" {
    @0.000
    sequence "old", loop: loop

    @5.000
    stop sequence "old"
    sequence "new"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Should parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Find the cue at 5.0s
    let cue_at_5 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 5.0).abs() < 0.01);
    assert!(cue_at_5.is_some(), "Should have a cue at 5.0s");
    let cue = cue_at_5.unwrap();

    // Should stop the old sequence
    assert!(
        cue.stop_sequences.contains(&"old".to_string()),
        "Should stop 'old' sequence at 5.0s"
    );

    // Should have effects from the new sequence
    assert!(
        !cue.effects.is_empty(),
        "Should have effects from 'new' sequence at 5.0s"
    );
}

// ===================================================================
// Multiple unnamed shows error
// ===================================================================

#[test]
fn test_multiple_unnamed_shows_error() {
    // When multiple shows are defined in a file, all must have explicit names.
    // Two shows without names should produce an error.
    let content = r#"
show {
    @0.000
    front_wash: static, color: "red"
}

show {
    @0.000
    front_wash: static, color: "blue"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_err(),
        "Multiple unnamed shows should produce an error"
    );

    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Show name is required when multiple shows are defined"),
        "Error should mention show name requirement: {}",
        error
    );
}

#[test]
fn test_single_unnamed_show_gets_default_name() {
    // A single show without an explicit name should be assigned "default".
    let content = r#"
show {
    @0.000
    front_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Single unnamed show should succeed: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    assert!(
        shows.contains_key("default"),
        "Single unnamed show should get name 'default'"
    );
}

// ===================================================================
// Layer commands with speed and intensity percentage parameters
// ===================================================================

#[test]
fn test_layer_command_master_with_speed_percent() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", layer: background

    @1.000
    master(layer: background, speed: 150%)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse master with speed percentage: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let cue_at_1 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01);
    assert!(cue_at_1.is_some(), "Should have a cue at 1.0s");
    let cue = cue_at_1.unwrap();

    assert_eq!(cue.layer_commands.len(), 1);
    assert_eq!(cue.layer_commands[0].command_type, LayerCommandType::Master);
    assert!(
        cue.layer_commands[0].speed.is_some(),
        "Master command should have speed"
    );
    let speed = cue.layer_commands[0].speed.unwrap();
    assert!(
        (speed - 1.5).abs() < 0.01,
        "Speed should be 1.5 (150%), got {}",
        speed
    );
}

#[test]
fn test_layer_command_master_with_intensity_percent() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", layer: foreground

    @1.000
    master(layer: foreground, intensity: 50%)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse master with intensity percentage: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let cue_at_1 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01);
    assert!(cue_at_1.is_some(), "Should have a cue at 1.0s");
    let cue = cue_at_1.unwrap();

    assert_eq!(cue.layer_commands.len(), 1);
    assert_eq!(cue.layer_commands[0].command_type, LayerCommandType::Master);
    assert!(
        cue.layer_commands[0].intensity.is_some(),
        "Master command should have intensity"
    );
    let intensity = cue.layer_commands[0].intensity.unwrap();
    assert!(
        (intensity - 0.5).abs() < 0.01,
        "Intensity should be 0.5 (50%), got {}",
        intensity
    );
}

#[test]
fn test_layer_command_release_with_time() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", layer: background

    @5.000
    release(layer: background, time: 2s)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse release with time: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let cue_at_5 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 5.0).abs() < 0.01);
    assert!(cue_at_5.is_some(), "Should have a cue at 5.0s");
    let cue = cue_at_5.unwrap();

    assert_eq!(cue.layer_commands.len(), 1);
    assert_eq!(
        cue.layer_commands[0].command_type,
        LayerCommandType::Release
    );
    assert!(
        cue.layer_commands[0].fade_time.is_some(),
        "Release should have fade_time"
    );
    let fade_time = cue.layer_commands[0].fade_time.unwrap();
    assert!(
        (fade_time.as_secs_f64() - 2.0).abs() < 0.01,
        "Fade time should be 2.0s, got {:?}",
        fade_time
    );
}

// ===================================================================
// Offset command without tempo section
// ===================================================================

#[test]
fn test_offset_command_without_tempo_in_show() {
    // Using offset without a tempo section should fail.
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red"

    offset 4 measures

    @0.000
    front_wash: static, color: "blue"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Offset without tempo should fail");

    let error = result.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Offset command requires a tempo section")
            || error.to_string().contains("tempo"),
        "Error should mention tempo requirement: {}",
        error
    );
}

// ===================================================================
// Sequence loop with pingpong mode (unsupported)
// ===================================================================

#[test]
fn test_sequence_loop_pingpong_error() {
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "seq", loop: pingpong
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_err(),
        "PingPong loop mode should error (not yet implemented)"
    );

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("PingPong") || error.to_string().contains("not yet implemented"),
        "Error should mention PingPong not implemented: {}",
        error
    );
}

#[test]
fn test_sequence_loop_random_error() {
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "seq", loop: random
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_err(),
        "Random loop mode should error (not yet implemented)"
    );

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Random") || error.to_string().contains("not yet implemented"),
        "Error should mention Random not implemented: {}",
        error
    );
}

// ===================================================================
// parse_sequence_loop_param: invalid non-numeric value
// ===================================================================

#[test]
fn test_sequence_loop_param_invalid_value() {
    // "abc" is not a valid sequence_param_value at the grammar level,
    // so the parser produces a DSL parsing error (not a runtime error).
    let content = r#"
sequence "seq" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "seq", loop: abc
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Invalid loop parameter should fail");

    // The error comes from the PEG parser, so it mentions the grammar expectation
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("parsing error") || error.to_string().contains("expected"),
        "Error should be a parsing error: {}",
        error
    );
}

// ===================================================================
// Content that looks like a show but fails parsing (analyze_parsing_failure)
// ===================================================================

#[test]
fn test_analyze_parsing_failure_with_show_keyword() {
    // Content that contains "show" but is not valid DSL should trigger
    // the analyze_parsing_failure path if no shows are parsed.
    let content = r#"show"#;

    let result = parse_light_shows(content);
    // This should either fail parsing or produce an error via analyze_parsing_failure
    assert!(
        result.is_err(),
        "Invalid content containing 'show' keyword should fail"
    );
}

// ===================================================================
// Layer command with midground layer
// ===================================================================

// ===================================================================
// Layer command with all parameter types (intensity, speed, time)
// ===================================================================

#[test]
fn test_layer_command_master_with_intensity_and_speed() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", layer: foreground

    @1.000
    master(layer: foreground, intensity: 50%, speed: 200%)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse master with intensity and speed: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let cue_at_1 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01)
        .expect("Should have cue at 1.0s");

    assert_eq!(cue_at_1.layer_commands.len(), 1);
    let cmd = &cue_at_1.layer_commands[0];
    assert_eq!(cmd.command_type, LayerCommandType::Master);
    assert!(cmd.intensity.is_some());
    assert!((cmd.intensity.unwrap() - 0.5).abs() < 1e-9);
    assert!(cmd.speed.is_some());
    assert!((cmd.speed.unwrap() - 2.0).abs() < 1e-9);
}

#[test]
fn test_layer_command_freeze_and_unfreeze() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: cycle, color: "red", color: "blue", layer: background

    @1.000
    freeze(layer: background)

    @2.000
    unfreeze(layer: background)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse freeze/unfreeze: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let freeze_cue = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01)
        .expect("Should have freeze cue at 1.0s");
    assert_eq!(
        freeze_cue.layer_commands[0].command_type,
        LayerCommandType::Freeze
    );

    let unfreeze_cue = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 2.0).abs() < 0.01)
        .expect("Should have unfreeze cue at 2.0s");
    assert_eq!(
        unfreeze_cue.layer_commands[0].command_type,
        LayerCommandType::Unfreeze
    );
}

#[test]
fn test_layer_command_clear_with_specific_layer() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", layer: foreground

    @1.000
    clear(layer: foreground)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse clear with specific layer: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let cue = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01)
        .expect("Should have cue at 1.0s");

    assert_eq!(cue.layer_commands[0].command_type, LayerCommandType::Clear);
    assert!(cue.layer_commands[0].layer.is_some());
}

// ===================================================================
// Layer command with numeric intensity (not percentage)
// ===================================================================

#[test]
fn test_layer_command_master_numeric_intensity() {
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", layer: foreground

    @1.000
    master(layer: foreground, intensity: 0.75)
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse master with numeric intensity: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    let cue = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01)
        .expect("Should have cue at 1.0s");

    let cmd = &cue.layer_commands[0];
    assert!((cmd.intensity.unwrap() - 0.75).abs() < 1e-9);
}

// ===================================================================
// Sequence loop parameter edge cases
// ===================================================================

#[test]
fn test_sequence_loop_once() {
    let content = r#"
sequence "my_seq" {
    @0.0
    wash: static, color: "red"

    @1.0
    wash: static, color: "blue"
}

show "Test" {
    @0.0
    sequence "my_seq", loop: once
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse sequence with loop: once: {:?}",
        result.err()
    );
}

#[test]
fn test_sequence_loop_numeric_count() {
    let content = r#"
sequence "my_seq" {
    @0.0
    wash: static, color: "red"

    @1.0
    wash: static, color: "blue"
}

show "Test" {
    @0.0
    sequence "my_seq", loop: 3
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse sequence with loop: 3: {:?}",
        result.err()
    );
}

// ===================================================================
// Show with multiple cues at different time formats
// ===================================================================

#[test]
fn test_show_with_seconds_and_minutes_format() {
    let content = r#"
show "Mixed Times" {
    @0.0
    wash: static, color: "red"

    @0:01.500
    wash: static, color: "green"

    @2.5s
    wash: static, color: "blue"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse mixed time formats: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Mixed Times").unwrap();
    assert_eq!(show.cues.len(), 3);
}

// ===================================================================
// Show with up/hold/down time on effects
// ===================================================================

#[test]
fn test_effect_with_duration() {
    let content = r#"
show "FadeTest" {
    @0.0
    wash: static, color: "red", duration: 2.0s
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse effect with duration: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("FadeTest").unwrap();
    assert_eq!(show.cues.len(), 1);
    assert_eq!(show.cues[0].effects.len(), 1);
}

// ===================================================================
// Effect types: rainbow, dimmer, pulse, chase, strobe via DSL
// ===================================================================

#[test]
fn test_dimmer_effect_via_dsl() {
    let content = r#"
show "Dimmer" {
    @0.0
    wash: dimmer, start: 0%, end: 100%, duration: 2s
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse dimmer effect: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Dimmer").unwrap();
    assert_eq!(show.cues.len(), 1);
}

#[test]
fn test_rainbow_effect_via_dsl() {
    let content = r#"
show "Rainbow" {
    @0.0
    wash: rainbow, speed: 2.0, saturation: 80%, brightness: 60%
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse rainbow effect: {:?}",
        result.err()
    );
}

#[test]
fn test_chase_effect_via_dsl() {
    let content = r#"
show "Chase" {
    @0.0
    wash: chase, pattern: snake, direction: clockwise, speed: 3.0, transition: fade
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse chase effect: {:?}",
        result.err()
    );
}

#[test]
fn test_pulse_effect_via_dsl() {
    let content = r#"
show "Pulse" {
    @0.0
    wash: pulse, base_level: 20%, intensity: 80%, frequency: 2.0, duration: 5s
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse pulse effect: {:?}",
        result.err()
    );
}

#[test]
fn test_strobe_effect_via_dsl() {
    let content = r#"
show "Strobe" {
    @0.0
    wash: strobe, rate: 15.0, duration: 3s
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse strobe effect: {:?}",
        result.err()
    );
}

// ===================================================================
// Inline loop in show cues
// ===================================================================

#[test]
fn test_inline_loop_in_show() {
    let content = r#"
show "Loop" {
    @0.0
    loop {
        @0.0
        wash: static, color: "red"
        @0.5
        wash: static, color: "blue"
    }, repeats: 3
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse inline loop: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Loop").unwrap();
    // 3 iterations × 2 cues = 6 cues
    assert!(
        show.cues.len() >= 3,
        "Should have expanded loop cues, got {}",
        show.cues.len()
    );
}

// ===================================================================
// Show with blend mode on effect
// ===================================================================

#[test]
fn test_effect_with_blend_mode() {
    let content = r#"
show "Blend" {
    @0.0
    wash: static, color: "red", layer: background

    @0.0
    wash: static, color: "blue", layer: foreground, blend: add
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse effects with blend mode: {:?}",
        result.err()
    );
}

// ===================================================================
// Show with stop_sequence command
// ===================================================================

#[test]
fn test_stop_sequence_in_show() {
    let content = r#"
sequence "running" {
    @0.0
    wash: static, color: "red"
    @1.0
    wash: static, color: "blue"
}

show "Test" {
    @0.0
    sequence "running", loop: loop

    @5.0
    stop sequence "running"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse stop_sequence: {:?}",
        result.err()
    );
}

// ===================================================================
// Global tempo with sequence
// ===================================================================

#[test]
fn test_global_tempo_with_sequence() {
    let content = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "beat_seq" {
    @1/1
    wash: static, color: "red"
    @2/1
    wash: static, color: "blue"
}

show "GlobalTempo" {
    @1/1
    sequence "beat_seq"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse with global tempo and sequence: {:?}",
        result.err()
    );
}

// ===================================================================
// Show with empty show content (no cues)
// ===================================================================

#[test]
fn test_show_with_no_cues() {
    let content = r#"
show "Empty" {
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Should parse empty show: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Empty").unwrap();
    assert!(show.cues.is_empty());
}
