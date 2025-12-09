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
fn test_measure_offset() {
    // Test that offset commands correctly adjust measure numbers
    // Measures 1-8 repeat once, so measure 9 should be at playback measure 17
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    offset 8 measures
    
    @1/1
    all_wash: static, color: "green"
    
    @8/1
    all_wash: static, color: "yellow"
    
    @9/1
    all_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with offset: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 measure = 2.0s
    // Measure 1, beat 1 = 0.0s + (0 measures * 2.0s) = 0.0s
    // After offset 8: measure 1 becomes playback measure 9 = 0.0s + (8 measures * 2.0s) = 16.0s
    // After offset 8: measure 8 becomes playback measure 16 = 0.0s + (15 measures * 2.0s) = 30.0s
    // After offset 8: measure 9 becomes playback measure 17 = 0.0s + (16 measures * 2.0s) = 32.0s

    assert!(show.cues.len() >= 4, "Should have at least 4 cues");

    // First cue at measure 1 = 0.0s
    let first_cue_time = show.cues[0].time;
    let expected_first = Duration::from_secs_f64(0.0);
    assert!(
        (first_cue_time.as_secs_f64() - expected_first.as_secs_f64()).abs() < 0.001,
        "First cue should be at measure 1 (0.0s), got {:?}",
        first_cue_time
    );

    // Second cue at measure 1 (with offset) = playback measure 9 = 16.0s
    let second_cue_time = show.cues[1].time;
    let expected_second = Duration::from_secs_f64(16.0);
    assert!(
        (second_cue_time.as_secs_f64() - expected_second.as_secs_f64()).abs() < 0.001,
        "Second cue should be at playback measure 9 (16.0s), got {:?}",
        second_cue_time
    );

    // Third cue at measure 8 (with offset) = playback measure 16 = 30.0s
    let third_cue_time = show.cues[2].time;
    let expected_third = Duration::from_secs_f64(30.0);
    assert!(
        (third_cue_time.as_secs_f64() - expected_third.as_secs_f64()).abs() < 0.001,
        "Third cue should be at playback measure 16 (30.0s), got {:?}",
        third_cue_time
    );

    // Fourth cue at measure 9 (with offset) = playback measure 17 = 32.0s
    let fourth_cue_time = show.cues[3].time;
    let expected_fourth = Duration::from_secs_f64(32.0);
    assert!(
        (fourth_cue_time.as_secs_f64() - expected_fourth.as_secs_f64()).abs() < 0.001,
        "Fourth cue should be at playback measure 17 (32.0s), got {:?}",
        fourth_cue_time
    );
}

#[test]
fn test_reset_measures() {
    // Test that reset_measures resets the offset back to 0
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    offset 8 measures
    
    @1/1
    all_wash: static, color: "green"
    
    reset_measures
    
    @1/1
    all_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with reset_measures: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 measure = 2.0s
    // First cue at measure 1 = 0.0s
    // Second cue at measure 1 (with offset 8) = playback measure 9 = 16.0s
    // Third cue at measure 1 (after reset) = 0.0s again

    assert!(show.cues.len() >= 3, "Should have at least 3 cues");

    let first_cue_time = show.cues[0].time;
    let second_cue_time = show.cues[1].time;
    let third_cue_time = show.cues[2].time;

    assert!(
        (first_cue_time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s, got {:?}",
        first_cue_time
    );

    assert!(
        (second_cue_time.as_secs_f64() - 16.0).abs() < 0.001,
        "Second cue should be at 16.0s (measure 9), got {:?}",
        second_cue_time
    );

    assert!(
        (third_cue_time.as_secs_f64() - 0.0).abs() < 0.001,
        "Third cue should be at 0.0s (after reset), got {:?}",
        third_cue_time
    );
}

#[test]
fn test_measure_offset_accumulation() {
    // Test that multiple offset commands accumulate
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    offset 4 measures
    
    @1/1
    all_wash: static, color: "green"
    
    offset 4 measures
    
    @1/1
    all_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with accumulating offsets: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 measure = 2.0s
    // First cue at measure 1 = 0.0s
    // After offset 4: measure 1 becomes playback measure 5 = 8.0s
    // After offset 8 (4+4): measure 1 becomes playback measure 9 = 16.0s

    assert!(show.cues.len() >= 3, "Should have at least 3 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 8.0).abs() < 0.001,
        "Second cue should be at 8.0s (measure 5)"
    );

    assert!(
        (show.cues[2].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Third cue should be at 16.0s (measure 9)"
    );
}

#[test]
fn test_measure_offset_in_sequence() {
    // Test that offset works correctly in sequences
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

sequence "verse" {
    @1/1
    all_wash: static, color: "blue"
    
    offset 8 measures
    
    @1/1
    all_wash: static, color: "green"
    
    @9/1
    all_wash: static, color: "red"
}

show "Test" {
    @1/1
    sequence "verse"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse sequence with offset: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Sequence is referenced at measure 1, so its @1/1 becomes measure 1
    // After offset 8 in sequence: @1/1 becomes playback measure 9
    // After offset 8 in sequence: @9/1 becomes playback measure 17
    // But these are relative to sequence start (measure 1), so:
    // First cue: measure 1 = 0.0s
    // Second cue: measure 9 = 16.0s
    // Third cue: measure 17 = 32.0s

    assert!(show.cues.len() >= 3, "Should have at least 3 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Second cue should be at 16.0s (measure 9)"
    );

    assert!(
        (show.cues[2].time.as_secs_f64() - 32.0).abs() < 0.001,
        "Third cue should be at 32.0s (measure 17)"
    );
}

#[test]
fn test_measure_offset_with_fractional_beats() {
    // Test that offset works with fractional beats
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1.5
    all_wash: static, color: "blue"
    
    offset 8 measures
    
    @1/2.5
    all_wash: static, color: "green"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with offset and fractional beats: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 beat = 0.5s
    // First cue: measure 1, beat 1.5 = 0.0s + (0 measures + 0.5 beats) * 0.5s = 0.25s
    // After offset 8: measure 1, beat 2.5 = (8 measures + 1.5 beats) * 0.5s = 16.0s + 0.75s = 16.75s

    assert!(show.cues.len() >= 2, "Should have at least 2 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.25).abs() < 0.001,
        "First cue should be at 0.25s"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 16.75).abs() < 0.001,
        "Second cue should be at 16.75s"
    );
}

#[test]
fn test_measure_offset_with_tempo_changes() {
    // Test that offset works correctly with tempo changes
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 {
    bpm: 60
    }
    ]
}

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    offset 8 measures
    
    @1/1
    all_wash: static, color: "green"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with offset and tempo change: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 measure = 2.0s
    // With offset 8 applied to tempo changes:
    // - Tempo change at score measure 8/1 + offset 8 = playback measure 16
    // - Second cue at score measure 1 + offset 8 = playback measure 9
    // Since the tempo change is at playback measure 16 (after the cue at measure 9),
    // the second cue is still at 120 BPM
    // Measures 1-8 at 120 BPM = 8 * 2.0s = 16.0s
    // Start of measure 9: 16.0s

    assert!(show.cues.len() >= 2, "Should have at least 2 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Second cue should be at 16.0s (measure 9 at 120 BPM, tempo change is at measure 16)"
    );
}

#[test]
fn test_measure_offset_in_same_cue() {
    // Test that when a cue has both a measure time and an offset command,
    // the offset does NOT apply to that cue's measure time, but DOES apply to subsequent cues.
    // This tests the scenario where offset is in the same cue definition as a measure time.
    let content = r#"tempo {
        start: 1.5s
    bpm: 160
    time_signature: 4/4
    changes: [
        @68/1 { bpm: 180 },
        @104/1 { bpm: 160 }
    ]
    }

show "Test" {
    @70/1
    all_wash: static, color: "blue"
    
    @74/1
    all_wash: static, color: "green"
    offset 5 measures
    
    @70/1
    all_wash: static, color: "red"
    
    @74/1
    all_wash: static, color: "yellow"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with offset in same cue: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Verify we have 4 cues
    assert!(show.cues.len() >= 4, "Should have at least 4 cues");

    // First cue at @70/1 (no offset yet)
    // Tempo changes at @68/1 to 180 BPM
    // Measures 1-67 at 160 BPM: 67 * 1.5s = 100.5s
    // Measures 68-70 at 180 BPM: 2 * 1.333s = 2.667s
    // Total: 100.5 + 2.667 = 103.167s
    // Plus start_offset 1.5s: 103.167 + 1.5 = 104.667s
    let first_cue_time = show.cues[0].time;
    let expected_first = Duration::from_secs_f64(104.666666667);
    assert!(
        (first_cue_time.as_secs_f64() - expected_first.as_secs_f64()).abs() < 0.01,
        "First cue should be at measure 70 (104.667s), got {:?} ({:.3}s)",
        first_cue_time,
        first_cue_time.as_secs_f64()
    );

    // Second cue at @74/1 (no offset yet)
    // Measures 1-67 @160 BPM: 100.5s
    // Measures 68-73 @180 BPM: 6 * 1.333s = 8.0s
    // Plus start_offset 1.5s => 110.0s
    let second_cue_time = show.cues[1].time;
    let expected_second = Duration::from_secs_f64(110.0);
    assert!(
        (second_cue_time.as_secs_f64() - expected_second.as_secs_f64()).abs() < 0.01,
        "Second cue should be at measure 74 (110.0s), got {:?} ({:.3}s)",
        second_cue_time,
        second_cue_time.as_secs_f64()
    );

    // Third cue at @70/1 with offset 5 measures = playback measure 75
    // Base @70/1 time: 104.667s, offset 5 measures at 180 BPM = 6.667s
    let third_cue_time = show.cues[2].time;
    let expected_third = Duration::from_secs_f64(111.333333333);
    assert!(
        (third_cue_time.as_secs_f64() - expected_third.as_secs_f64()).abs() < 0.01,
        "Third cue should be at playback measure 75 (~111.33s) after offset 5, got {:?} ({:.3}s)",
        third_cue_time,
        third_cue_time.as_secs_f64()
    );

    // Fourth cue at @74/1 with offset 5 measures = playback measure 79
    // Base @74/1 time: 110.0s, offset still 5 measures at 180 BPM = 6.667s => ~116.67s
    let fourth_cue_time = show.cues[3].time;
    let expected_fourth = Duration::from_secs_f64(116.666666667);
    assert!(
        (fourth_cue_time.as_secs_f64() - expected_fourth.as_secs_f64()).abs() < 0.01,
        "Fourth cue should be at playback measure 79 (~116.67s) after offset 5, got {:?} ({:.3}s)",
        fourth_cue_time,
        fourth_cue_time.as_secs_f64()
    );
}

#[test]
fn test_measure_offset_with_measure_time_in_same_cue() {
    // Test that when a cue has both a measure time AND an offset command in the same
    // cue definition, the offset does NOT apply to the current cue's measure time.
    // The offset should only affect subsequent cues. The effect should still be included.
    let content = r#"tempo {
        start: 0.0s
    bpm: 120
    time_signature: 4/4
    }

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    @5/1
    all_wash: static, color: "green"
    offset 10 measures
    
    @1/1
    all_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with offset and measure time in same cue: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // Verify we have 3 cues
    assert!(show.cues.len() >= 3, "Should have at least 3 cues");

    // At 120 BPM in 4/4: 1 measure = 2.0s

    // First cue at @1/1 (no offset) = 0.0s
    let first_cue_time = show.cues[0].time;
    let expected_first = Duration::from_secs_f64(0.0);
    assert!(
        (first_cue_time.as_secs_f64() - expected_first.as_secs_f64()).abs() < 0.001,
        "First cue should be at measure 1 (0.0s), got {:?} ({:.3}s)",
        first_cue_time,
        first_cue_time.as_secs_f64()
    );
    // Verify first cue has the effect
    assert!(
        !show.cues[0].effects.is_empty(),
        "First cue should have effects"
    );

    // Second cue at @5/1 with offset 10 measures in the SAME cue
    // IMPORTANT: The offset should NOT apply to this cue's measure time.
    // So @5/1 should be at playback measure 5 = 4 measures * 2.0s = 8.0s
    let second_cue_time = show.cues[1].time;
    let expected_second = Duration::from_secs_f64(8.0);
    assert!(
        (second_cue_time.as_secs_f64() - expected_second.as_secs_f64()).abs() < 0.001,
        "Second cue should be at playback measure 5 (8.0s), offset should NOT apply to current cue, got {:?} ({:.3}s)",
        second_cue_time,
        second_cue_time.as_secs_f64()
    );
    // Verify second cue has the effect (this is the key test - effect should not be lost)
    assert!(
        !show.cues[1].effects.is_empty(),
        "Second cue should have effects even though it has an offset command"
    );

    // Third cue at @1/1 (with offset 10 from previous cue) = playback measure 11 = 10 measures * 2.0s = 20.0s
    let third_cue_time = show.cues[2].time;
    let expected_third = Duration::from_secs_f64(20.0);
    assert!(
        (third_cue_time.as_secs_f64() - expected_third.as_secs_f64()).abs() < 0.001,
        "Third cue should be at playback measure 11 (20.0s) with offset 10, got {:?} ({:.3}s)",
        third_cue_time,
        third_cue_time.as_secs_f64()
    );
}

#[test]
fn test_offset_timing_at_180_bpm_with_tempo_change() {
    // Test the exact scenario: @70/1, @74/1 with offset 5, @70/1 at 180 BPM
    // This verifies that the offset is applied correctly and timing is accurate
    // when there's a tempo change from 160 to 180 BPM at @68/1
    let content = r#"tempo {
        start: 1.5s
    bpm: 160
    time_signature: 4/4
    changes: [
        @68/1 { bpm: 180 },
        @104/1 { bpm: 160 }
    ]
    }

show "Test" {
    @70/1
    all_wash: static, color: "blue"
    
    @74/1
    all_wash: static, color: "green"
    offset 5 measures
    
    @70/1
    all_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse show: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    assert!(show.cues.len() >= 3, "Should have at least 3 cues");

    // At 180 BPM in 4/4: 1 measure = 4 beats * (60/180) = 1.333... seconds
    // Measure 74 to measure 75 = 1 measure = 1.333 seconds

    let second_cue_time = show.cues[1].time; // @74/1
    let third_cue_time = show.cues[2].time; // @70/1 with offset 5 (should be measure 75)

    println!(
        "Second cue (@74/1) time: {:.3}s",
        second_cue_time.as_secs_f64()
    );
    println!(
        "Third cue (@70/1 with offset 5) time: {:.3}s",
        third_cue_time.as_secs_f64()
    );

    // Also compute directly from tempo map for visibility
    let tm = show.tempo_map.as_ref().unwrap();
    let calc_74 = tm
        .measure_to_time_with_offset(74, 1.0, 0, 0.0)
        .unwrap()
        .as_secs_f64();
    let calc_70_off5 = tm
        .measure_to_time_with_offset(70, 1.0, 5, 0.0)
        .unwrap()
        .as_secs_f64();
    println!(
        "Calc tempo map: @74/1 = {:.3}s, @70/1 (offset 5) = {:.3}s, diff = {:.3}s",
        calc_74,
        calc_70_off5,
        calc_70_off5 - calc_74
    );

    let time_diff = third_cue_time.as_secs_f64() - second_cue_time.as_secs_f64();
    let expected_diff = 1.333333333; // 1 measure at 180 BPM

    // Calculate what measure the third cue is actually at based on the time difference
    let actual_measures = time_diff / 1.333333333; // measures at 180 BPM
    println!(
        "Time difference: {:.3}s = {:.3} measures at 180 BPM (expected: 1.0 measure)",
        time_diff, actual_measures
    );

    assert!(
        (time_diff - expected_diff).abs() < 0.01,
        "Time difference between @74/1 and second @70/1 (with offset 5) should be ~1.333s (1 measure at 180 BPM), got {:.3}s (difference: {:.3}s, actual: {:.3} measures)",
        time_diff,
        time_diff - expected_diff,
        actual_measures
    );
}

#[test]
fn test_measure_offset_at_start() {
    // Test that offset can be in the first cue
    let content = r#"tempo {
        start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1
    offset 8 measures
    
    @1/1
    all_wash: static, color: "blue"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with offset in first cue: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // First cue at @1/1 = 0.0s (parsed before offset command), then offset is set to 8
    // Second cue at @1/1 with offset 8 = playback measure 9 = 16.0s

    assert!(show.cues.len() >= 2, "Should have at least 2 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s (before offset takes effect)"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Second cue should be at 16.0s (measure 9, after offset)"
    );
}

#[test]
fn test_measure_offset_reset_and_reoffset() {
    // Test reset followed by another offset
    // Note: offset commands affect subsequent cues, not the current one
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    offset 8 measures
    
    @1/1
    all_wash: static, color: "green"
    
    @1/1
    all_wash: static, color: "yellow"
    reset_measures
    offset 4 measures
    
    @1/1
    all_wash: static, color: "red"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with reset and reoffset: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // First cue: measure 1 = 0.0s, then offset becomes 8
    // Second cue: measure 1 with offset 8 = measure 9 = 16.0s
    // Third cue: measure 1 with offset 8 (still) = measure 9 = 16.0s, then reset and offset 4
    // Fourth cue: measure 1 with offset 4 (after reset) = measure 5 = 8.0s

    assert!(show.cues.len() >= 4, "Should have at least 4 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Second cue should be at 16.0s (measure 9)"
    );

    assert!(
        (show.cues[2].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Third cue should be at 16.0s (measure 9, before reset takes effect)"
    );

    let fourth_cue_time = show.cues[3].time.as_secs_f64();
    assert!(
        (fourth_cue_time - 8.0).abs() < 0.001,
        "Fourth cue should be at 8.0s (measure 5 after reset and offset 4), got {}",
        fourth_cue_time
    );
}

#[test]
fn test_measure_offset_with_alternate_endings_scenario() {
    // Test the original use case: measures 1-8 repeat twice, measure 9 is alternate ending
    // This simulates: measures 1-8 play, then 1-8 again, but measure 9 replaces measure 8 on 2nd repeat
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Test" {
    @1/1
    all_wash: static, color: "blue"
    
    @8/1
    all_wash: static, color: "yellow"
    
    offset 8 measures
    
    @1/1
    all_wash: static, color: "green"
    
    @7/1
    all_wash: static, color: "orange"
    
    @9/1
    all_wash: static, color: "red"
    
    @10/1
    all_wash: static, color: "purple"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse show with alternate endings scenario: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 120 BPM in 4/4: 1 measure = 2.0s
    // First playthrough: measures 1-8
    //   @1/1 = 0.0s (measure 1)
    //   @8/1 = 14.0s (measure 8)
    // Second playthrough (after offset 8): measures 1-8 again, but measure 9 replaces 8
    //   @1/1 = 16.0s (playback measure 9)
    //   @7/1 = 28.0s (playback measure 15 = 14 measures * 2.0s)
    //   @9/1 = 32.0s (playback measure 17 = 16 measures * 2.0s) - this is the alternate ending
    //   @10/1 = 34.0s (playback measure 18 = 17 measures * 2.0s) - continues after repeat

    assert!(show.cues.len() >= 6, "Should have at least 6 cues");

    assert!(
        (show.cues[0].time.as_secs_f64() - 0.0).abs() < 0.001,
        "First cue should be at 0.0s"
    );

    assert!(
        (show.cues[1].time.as_secs_f64() - 14.0).abs() < 0.001,
        "Second cue should be at 14.0s (measure 8)"
    );

    assert!(
        (show.cues[2].time.as_secs_f64() - 16.0).abs() < 0.001,
        "Third cue should be at 16.0s (measure 9, second repeat)"
    );

    assert!(
        (show.cues[3].time.as_secs_f64() - 28.0).abs() < 0.001,
        "Fourth cue should be at 28.0s (measure 15)"
    );

    assert!(
        (show.cues[4].time.as_secs_f64() - 32.0).abs() < 0.001,
        "Fifth cue should be at 32.0s (measure 17, alternate ending)"
    );

    assert!(
        (show.cues[5].time.as_secs_f64() - 34.0).abs() < 0.001,
        "Sixth cue should be at 34.0s (measure 18, after repeat)"
    );
}
