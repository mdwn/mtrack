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
use crate::lighting::parser::grammar::{LightingParser, Rule};
use pest::Parser;

// ========================================================================
// TEMPO-BASED TIMING TESTS
// ========================================================================

#[test]
fn test_invalid_measure_time_formats() {
    // These should fail to parse
    let invalid_cases = vec![
        "@invalid/1",
        "@1/invalid",
        "@1:1", // wrong separator
        "@/1",  // missing measure
        "@1/",  // missing beat
    ];

    for case in invalid_cases {
        let result = LightingParser::parse(Rule::measure_time, case);
        assert!(
            result.is_err(),
            "Invalid measure time format {} should fail to parse",
            case
        );
    }
}

#[test]
fn test_invalid_time_signatures() {
    // Test that syntactically invalid time signatures fail to parse
    // Note: Semantically invalid but syntactically valid values (like 0/4)
    // will parse successfully and need semantic validation in the implementation

    let syntactically_invalid = vec![
        "abc/4", // non-numeric numerator
        "4/xyz", // non-numeric denominator
        "4",     // missing denominator
        "/4",    // missing numerator
        "4/",    // missing denominator with slash
        "4:4",   // wrong separator
        "4 / 4", // spaces (not allowed in atomic rule)
        "-4/4",  // negative numerator (not ASCII_DIGIT)
        "4/-4",  // negative denominator (not ASCII_DIGIT)
    ];

    for sig in syntactically_invalid {
        let content = format!(
            r#"tempo {{
    start: 0.0s
    bpm: 120
    time_signature: {}
}}"#,
            sig
        );

        let result = LightingParser::parse(Rule::tempo, &content);
        assert!(
            result.is_err(),
            "Syntactically invalid time signature {} should fail to parse",
            sig
        );
    }

    // These are syntactically valid but semantically invalid
    // The grammar will accept them, but implementation should reject them
    let semantically_invalid = vec![
        "0/4", // zero numerator (valid syntax, invalid semantics)
        "4/0", // zero denominator (valid syntax, invalid semantics)
        "0/0", // both zero (valid syntax, invalid semantics)
    ];

    for sig in semantically_invalid {
        let content = format!(
            r#"tempo {{
    start: 0.0s
    bpm: 120
    time_signature: {}
}}"#,
            sig
        );

        let result = LightingParser::parse(Rule::tempo, &content);
        // Grammar will parse these successfully
        assert!(
            result.is_ok(),
            "Semantically invalid time signature {} parses successfully (needs runtime validation)",
            sig
        );
    }

    println!(
        "Note: Grammar validation is syntax-only. Semantic validation \
     (zero/negative values) should be done in the implementation."
    );
}

#[test]
fn test_time_signature_change_with_invalid_position() {
    // Test that time signature change requires a valid measure position
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @invalid { time_signature: 3/4 }
    ]
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    assert!(
        result.is_err(),
        "Time signature change with invalid position should fail"
    );
}

#[test]
fn test_combined_invalid_changes() {
    // Test various invalid combinations
    // This documents that the grammar accepts syntactically valid but semantically invalid values

    // Negative BPM - not actually possible since grammar uses ASCII_DIGIT+
    // which doesn't include '-'
    let content1 = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { bpm: -120 }
    ]
}"#;
    let _result1 = LightingParser::parse(Rule::tempo, content1);
    // This will actually fail to parse since '-' is not part of ASCII_DIGIT

    // Zero time signature in change - syntactically valid, semantically invalid
    let content2 = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { time_signature: 0/4 }
    ]
}"#;
    let result2 = LightingParser::parse(Rule::tempo, content2);
    // Grammar will parse this successfully, but semantic validation should reject it
    assert!(result2.is_ok(), "Syntactically valid but semantically invalid time signature should parse (needs runtime validation)");

    // These tests document that grammar validation is separate from semantic validation
    println!(
        "Note: Grammar accepts syntactically valid but semantically invalid values. \
     Implementation should add semantic validation for: zero BPM, \
     zero numerator/denominator in time signatures."
    );
}

// ========================================================================
// BEAT POSITION VALIDATION TESTS (SEMANTIC)
// ========================================================================
// These tests document that the grammar will parse beat positions that
// may be semantically invalid for the current time signature.
// The implementation should add runtime validation.

#[test]
fn test_beat_position_exceeds_time_signature() {
    // In 4/4 time, beat 5 doesn't exist (only beats 1-4)
    // Grammar will parse this, but implementation should validate
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Invalid Beat Position" {
    @1/1
    front_wash: static color: "blue"
    
    @2/5
    back_wash: static color: "red"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(
        result.is_ok(),
        "Beat position exceeding time signature parses (needs semantic validation)"
    );
    println!(
        "Note: @2/5 in 4/4 time is semantically invalid (only 4 beats per measure). \
         Implementation should validate beat positions against time signature."
    );
}

#[test]
fn test_beat_position_valid_after_time_signature_change() {
    // Beat 5 is invalid in 4/4 but valid in 6/8
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { time_signature: 6/8 }
    ]
}

show "Beat Valid After Change" {
    @1/1
    front_wash: static color: "blue"
    
    @1/4
    back_wash: static color: "red"
    
    @8/1
    front_wash: static color: "green"
    
    @9/5
    back_wash: static color: "yellow"
    
    @9/6
    front_wash: static color: "purple"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate:\n\
         - @1/4 is valid in 4/4 time\n\
         - @9/5 is valid in 6/8 time (after change at @8/1)\n\
         - @9/6 is valid in 6/8 time"
    );
}

#[test]
fn test_beat_position_invalid_after_time_signature_change() {
    // Beat 4 is valid in 4/4 but invalid in 3/4
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { time_signature: 3/4 }
    ]
}

show "Beat Invalid After Change" {
    @1/1
    front_wash: static color: "blue"
    
    @1/4
    back_wash: static color: "red"
    
    @8/1
    front_wash: static color: "green"
    
    @9/4
    back_wash: static color: "yellow"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate:\n\
         - @1/4 is valid in 4/4 time\n\
         - @9/4 is INVALID in 3/4 time (only 3 beats per measure)\n\
         - Should raise semantic error for @9/4"
    );
}

#[test]
fn test_beat_zero_is_invalid() {
    // Beat 0 doesn't exist (beats are 1-indexed)
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Beat Zero" {
    @1/0
    front_wash: static color: "blue"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(
        result.is_ok(),
        "Beat 0 parses syntactically (needs semantic validation)"
    );
    println!(
        "Note: @1/0 is semantically invalid (beats are 1-indexed). \
         Implementation should reject beat positions < 1."
    );
}

#[test]
fn test_fractional_beat_exceeds_time_signature() {
    // Beat 4.5 in 4/4 time means halfway between beat 4 and 5
    // But beat 5 doesn't exist in 4/4 time
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Fractional Beat Beyond Time Sig" {
    @1/4.5
    front_wash: static color: "blue"
    
    @2/4.9
    back_wash: static color: "red"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate fractional beats:\n\
         - @1/4.5 is INVALID in 4/4 (would be halfway to non-existent beat 5)\n\
         - @1/4.0 or @1/4 would be valid\n\
         - Beat values should be in range [1, time_sig_numerator]"
    );
}

#[test]
fn test_beat_position_edge_cases_at_boundary() {
    // Test beat positions right at the boundary of valid beats
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Boundary Cases" {
    @1/1.0
    front_wash: static color: "blue"
    
    @1/4.0
    back_wash: static color: "red"
    
    @1/4.999
    side_wash: static color: "green"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate:\n\
         - @1/1.0 is valid (beat 1)\n\
         - @1/4.0 is valid (beat 4)\n\
         - @1/4.999 is valid (almost at the end of beat 4)\n\
         - @1/5.0 would be INVALID (beat 5 doesn't exist)"
    );
}

#[test]
fn test_multiple_time_sig_changes_with_beat_validation() {
    // Complex scenario with multiple time signature changes
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @5/1 { time_signature: 3/4 },
    @10/1 { time_signature: 6/8 },
    @15/1 { time_signature: 5/4 }
    ]
}

show "Complex Time Sig Changes" {
    @1/4
    front_wash: static color: "blue"
    
    @6/3
    back_wash: static color: "red"
    
    @11/6
    side_wash: static color: "green"
    
    @16/5
    top_wash: static color: "yellow"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate beat positions across time signature changes:\n\
         - @1/4 is valid in 4/4 time\n\
         - @6/3 is valid in 3/4 time (after change at @5/1)\n\
         - @11/6 is valid in 6/8 time (after change at @10/1)\n\
         - @16/5 is valid in 5/4 time (after change at @15/1)"
    );
}

#[test]
fn test_tempo_change_position_respects_time_signature() {
    // Tempo change at a beat position that's invalid for current time signature
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 3/4
    changes: [
    @8/4 { bpm: 140 }
    ]
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate tempo change positions:\n\
     - @8/4 is INVALID in 3/4 time (only 3 beats per measure)\n\
     - Tempo changes should respect the active time signature"
    );
}

#[test]
fn test_time_signature_change_position_respects_current_time_sig() {
    // Time signature change at a beat position that's invalid for current time signature
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 3/4
    changes: [
    @8/4 { time_signature: 4/4 }
    ]
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    assert!(result.is_ok(), "Grammar should parse successfully");
    println!(
        "Note: Implementation should validate time signature change positions:\n\
     - @8/4 is INVALID in 3/4 time (only 3 beats per measure)\n\
     - Time signature changes should occur at valid positions in the CURRENT time signature"
    );
}

// ========================================================================
// ADDITIONAL COVERAGE TESTS
// ========================================================================

#[test]
fn test_tempo_changes_with_absolute_time() {
    // Test tempo changes using absolute time (@MM:SS.mmm) instead of measures
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @00:30.000 { bpm: 140 },
    @01:00.500 { bpm: 160, transition: snap },
    @01:30.000 { time_signature: 3/4 }
    ]
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    if let Err(e) = &result {
        println!("Tempo changes with absolute time parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Tempo changes with absolute time positions should parse"
    );
}

#[test]
fn test_fractional_transition_durations() {
    // Test fractional measure and beat transitions
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { bpm: 140, transition: 2.5 },
    @16/1 { bpm: 160, transition: 1.5m },
    @24/1 { bpm: 180, transition: 0.25m }
    ]
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    if let Err(e) = &result {
        println!("Fractional transition durations parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Fractional measure and beat transitions should parse"
    );
}

#[test]
fn test_empty_tempo_content() {
    // Test tempo section with no content
    let content = r#"tempo {
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    assert!(
        result.is_ok(),
        "Empty tempo section should parse (fields are optional)"
    );
    println!(
        "Note: Empty tempo section is syntactically valid but semantically incomplete. \
     Implementation should require at least bpm and time_signature."
    );
}

#[test]
fn test_measure_zero_is_invalid() {
    // Measure 0 doesn't exist (measures are 1-indexed)
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Measure Zero" {
    @0/1
    front_wash: static color: "blue"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(
        result.is_ok(),
        "Measure 0 parses syntactically (needs semantic validation)"
    );
    println!(
        "Note: @0/1 is semantically invalid (measures are 1-indexed). \
         Implementation should reject measure positions < 1."
    );
}

#[test]
fn test_negative_measure_numbers() {
    // Negative measure numbers don't make sense
    let content = r#"show "Negative Measure" {
    @-1/1
    front_wash: static color: "blue"
}"#;

    let result = LightingParser::parse(Rule::file, content);
    // This should fail to parse since ASCII_DIGIT doesn't include '-'
    assert!(
        result.is_err(),
        "Negative measure numbers should fail to parse"
    );
}

#[test]
fn test_tempo_change_with_ss_mmm_format() {
    // Test SS.mmm format (without MM:) in tempo changes
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @30.500 { bpm: 140 },
    @45.0 { bpm: 160 }
    ]
}"#;

    let result = LightingParser::parse(Rule::tempo, content);
    if let Err(e) = &result {
        println!("Tempo change with SS.mmm format parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Tempo changes with SS.mmm time format should parse"
    );
}
