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

use crate::lighting::parser::*;
use std::time::Duration;

#[test]
fn test_tempo_changes_with_comments() {
    // Test that comments after tempo change parameters work
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @10.000 {
            bpm: 140
            # Comment after BPM
            time_signature: 6/4
            # Comment after time signature
        }
        @20.000 {
            bpm: 160
            transition: 2m
            # Comment after transition
        }
    ]
}

show "Test" {
    @0.000
    front_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse tempo changes with comments: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert!(show.tempo_map.is_some());
    
    let tempo_map = show.tempo_map.as_ref().unwrap();
    assert_eq!(tempo_map.changes.len(), 2);
    
    // First change should have BPM 140 and time signature 6/4
    let first_change = &tempo_map.changes[0];
    assert!(first_change.bpm.is_some());
    assert!((first_change.bpm.unwrap() - 140.0).abs() < 0.001);
    assert!(first_change.time_signature.is_some());
    let ts = first_change.time_signature.unwrap();
    assert_eq!(ts.numerator, 6);
    assert_eq!(ts.denominator, 4);
}

#[test]
fn test_time_signature_with_whitespace() {
    // Test that time signatures with whitespace parse correctly
    let content = r#"
tempo {
    bpm: 120
    time_signature: 6 / 4
    changes: [
        @10.000 {
            time_signature: 3 / 4
        }
    ]
}

show "Test" {
    @0.000
    front_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse time signature with whitespace: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert!(show.tempo_map.is_some());
    
    let tempo_map = show.tempo_map.as_ref().unwrap();
    let initial_ts = tempo_map.initial_time_sig;
    assert_eq!(initial_ts.numerator, 6);
    assert_eq!(initial_ts.denominator, 4);
    
    assert_eq!(tempo_map.changes.len(), 1);
    let change = &tempo_map.changes[0];
    assert!(change.time_signature.is_some());
    let ts = change.time_signature.unwrap();
    assert_eq!(ts.numerator, 3);
    assert_eq!(ts.denominator, 4);
}

#[test]
fn test_offset_command_with_comments() {
    // Test that comments after offset commands work
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    offset 2 measures
    # Comment after offset
    
    @2.000
    back_wash: static, color: "blue"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse offset command with comment: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert_eq!(show.cues.len(), 3);
}

#[test]
fn test_stop_sequence_with_comments() {
    // Test that comments after stop sequence commands work
    let content = r#"
sequence "seq1" {
    @0.000
    front_wash: static, color: "red"
}

show "Test" {
    @0.000
    sequence "seq1", loop: 3
    
    @10.000
    stop sequence "seq1"
    # Comment after stop sequence
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse stop sequence with comment: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    
    // Find the cue at 10 seconds with stop_sequences
    let stop_cue = show.cues.iter()
        .find(|c| c.time == Duration::from_secs(10) && !c.stop_sequences.is_empty());
    
    assert!(
        stop_cue.is_some(),
        "Should have a cue at 10 seconds with stop_sequences"
    );
    
    let stop_cue = stop_cue.unwrap();
    assert!(stop_cue.stop_sequences.contains(&"seq1".to_string()));
}

#[test]
fn test_group_names_with_whitespace() {
    // Test that group names with whitespace are trimmed correctly
    let content = r#"
show "Test" {
    @0.000
    front_wash, back_wash: static, color: "red", dimmer: 50%
    # Comment after group list
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse group names: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert_eq!(show.cues.len(), 1);
    
    let cue = &show.cues[0];
    assert_eq!(cue.effects.len(), 1);
    let effect = &cue.effects[0];
    assert_eq!(effect.groups.len(), 2);
    assert!(effect.groups.contains(&"front_wash".to_string()));
    assert!(effect.groups.contains(&"back_wash".to_string()));
}

#[test]
fn test_percentage_parameters_with_whitespace() {
    // Test that percentage parameters with whitespace parse correctly
    let content = r#"
show "Test" {
    @0.000
    front_wash: static, color: "red", dimmer: 50 %
    # Comment after percentage
    
    @1.000
    back_wash: static, color: "blue", dimmer: 75%
    # Another comment
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse percentage with whitespace: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert_eq!(show.cues.len(), 2);
}

#[test]
fn test_duration_parameters_with_whitespace() {
    // Test that duration parameters with whitespace parse correctly
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @0.000
    front_wash: static, color: "red", hold_time: 2 s
    # Comment after duration
    
    @1.000
    back_wash: static, color: "blue", hold_time: 500 ms
    # Another comment
    
    @2.000
    strobe_lights: strobe, hold_time: 4 beats
    # Comment after beats
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse duration with whitespace: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert_eq!(show.cues.len(), 3);
    
    // Verify durations were parsed correctly
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.effects.len(), 1);
    let effect = &first_cue.effects[0];
    assert!(effect.hold_time.is_some());
    assert_eq!(effect.hold_time.unwrap(), Duration::from_secs(2));
}

#[test]
fn test_layer_command_percentage_with_whitespace() {
    // Test that layer command percentage parameters with whitespace parse correctly
    let content = r#"
show "Test" {
    @0.000
    master(layer: foreground, intensity: 50 %)
    # Comment after percentage
    
    @1.000
    master(layer: background, speed: 200 %)
    # Another comment
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse layer command percentage with whitespace: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert_eq!(show.cues.len(), 2);
    
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.layer_commands.len(), 1);
    let master_cmd = &first_cue.layer_commands[0];
    assert!(master_cmd.intensity.is_some());
    assert!((master_cmd.intensity.unwrap() - 0.5).abs() < 0.001);
    
    let second_cue = &show.cues[1];
    assert_eq!(second_cue.layer_commands.len(), 1);
    let master_cmd = &second_cue.layer_commands[0];
    assert!(master_cmd.speed.is_some());
    assert!((master_cmd.speed.unwrap() - 2.0).abs() < 0.001);
}

#[test]
fn test_tempo_transition_with_whitespace() {
    // Test that tempo transition measures with whitespace parse correctly
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
        @10.000 {
            bpm: 140
            transition: 2 m
            # Comment after transition
        }
    ]
}

show "Test" {
    @0.000
    front_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse tempo transition with whitespace: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").expect("Show 'Test' should exist");
    assert!(show.tempo_map.is_some());
    
    let tempo_map = show.tempo_map.as_ref().unwrap();
    assert_eq!(tempo_map.changes.len(), 1);
    let change = &tempo_map.changes[0];
    
    // Verify transition was parsed (should be Measures variant)
    match change.transition {
        crate::lighting::tempo::TempoTransition::Measures(measures, _) => {
            assert!((measures - 2.0).abs() < 0.001);
        }
        _ => panic!("Expected Measures transition"),
    }
}

#[test]
fn test_comprehensive_comments_and_whitespace() {
    // Comprehensive test with comments and whitespace in various places
    let content = r#"
tempo {
    bpm: 120
    # Comment in tempo section
    time_signature: 4 / 4
    changes: [
        @10.000 {
            bpm: 140
            # Comment after BPM
            time_signature: 6 / 4
            transition: 2 m
        }
    ]
}

sequence "test_seq" {
    @0.000
    front_wash: static, color: "red"
    # Comment in sequence
}

show "Test Show" {
    @0.000
    front_wash, back_wash: static, color: "red", dimmer: 50 %
    # Comment after effect
    
    @1.000
    sequence "test_seq", loop: 3
    # Comment after sequence loop
    
    @2.000
    clear(layer: foreground)
    # Comment after layer command
    
    @3.000
    offset 2 measures
    # Comment after offset
    
    @4.000
    stop sequence "test_seq"
    # Comment after stop sequence
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse comprehensive test with comments and whitespace: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test Show").expect("Show 'Test Show' should exist");
    
    // Should have multiple cues
    assert!(show.cues.len() >= 4);
    
    // Verify tempo map was parsed
    assert!(show.tempo_map.is_some());
    let tempo_map = show.tempo_map.as_ref().unwrap();
    assert_eq!(tempo_map.changes.len(), 1);
}

