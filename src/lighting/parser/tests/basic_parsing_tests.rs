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
use crate::lighting::effects::Color;
use crate::lighting::parser::utils::parse_time_string;
use crate::lighting::parser::*;
use std::time::Duration;

#[test]
fn test_parse_multiple_shows() {
    let content = r#"show "Show 1" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}

show "Show 2" {
    @00:00.000
    back_wash: static color: "red", dimmer: 80%
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());
    let shows = result.unwrap();
    println!("Found {} shows", shows.len());
    assert_eq!(shows.len(), 2);
    assert!(shows.contains_key("Show 1"));
    assert!(shows.contains_key("Show 2"));
}

#[test]
fn test_parse_invalid_syntax() {
    let content = r#"show "Invalid Show" {
    @00:00.000
    front_wash: invalid_effect color: "blue"
}"#;

    let result = parse_light_shows(content);
    assert!(result.is_err());
}

#[test]
fn test_parse_malformed_timing() {
    let content = r#"show "Invalid Timing" {
    @invalid_time
    front_wash: static color: "blue", dimmer: 60%
}"#;

    let result = parse_light_shows(content);
    assert!(result.is_err());
}

#[test]
fn test_parse_empty_show() {
    let content = r#"show "Empty Show" {
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parse error: {}", e);
    }
    assert!(result.is_ok());
    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    let show = &shows["Empty Show"];
    assert_eq!(show.cues.len(), 0);
}

#[test]
fn test_color_parsing() {
    // Test hex colors
    let red = Color::from_hex("#ff0000").unwrap();
    assert_eq!(red.r, 255);
    assert_eq!(red.g, 0);
    assert_eq!(red.b, 0);

    // Test named colors
    let blue = Color::from_name("blue").unwrap();
    assert_eq!(blue.r, 0);
    assert_eq!(blue.g, 0);
    assert_eq!(blue.b, 255);

    // Test invalid hex
    assert!(Color::from_hex("invalid").is_err());

    // Test invalid color name
    assert!(Color::from_name("invalid").is_err());
}

#[test]
fn test_time_parsing() {
    // Test MM:SS.mmm format
    let time1 = parse_time_string("01:30.500").unwrap();
    assert_eq!(time1.as_millis(), 90500);

    // Test SS.mmm format
    let time2 = parse_time_string("30.500").unwrap();
    assert_eq!(time2.as_millis(), 30500);

    // Test edge cases
    let time3 = parse_time_string("00:00.000").unwrap();
    assert_eq!(time3.as_millis(), 0);
}

#[test]
fn test_parse_crossfade_example() {
    let content = r#"show "Crossfade Test" {
    @00:00.000
    front_wash: static color: "blue", up_time: 2s, down_time: 1s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);

    let show = shows.get("Crossfade Test").unwrap();
    assert_eq!(show.name, "Crossfade Test");
    assert_eq!(show.cues.len(), 1);

    let cue = &show.cues[0];
    assert_eq!(cue.time, Duration::from_millis(0));
    assert_eq!(cue.effects.len(), 1);

    let effect = &cue.effects[0];
    assert_eq!(effect.groups, vec!["front_wash"]);
    assert_eq!(effect.up_time, Some(Duration::from_secs(2)));
    assert_eq!(effect.down_time, Some(Duration::from_secs(1)));
    println!(
        "Timing parsing works! up_time: {:?}, down_time: {:?}",
        effect.up_time, effect.down_time
    );
}

#[test]
fn test_parse_zero_fade() {
    let content = r#"show "Zero Fade Test" {
    @00:00.000
    front_wash: static color: "blue", up_time: 0s, down_time: 0s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Zero Fade Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_secs(0)));
    assert_eq!(effect.down_time, Some(Duration::from_secs(0)));
    println!(
        "Zero timing parsing works! up_time: {:?}, down_time: {:?}",
        effect.up_time, effect.down_time
    );
}

#[test]
fn test_parse_layering_partial() {
    let content = r#"show "Effect Layering Demo" {
    @00:00.000
    # Background layer: Static blue color with 2 second fade in
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace, up_time: 2s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Effect Layering Demo").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_secs(2)));
    println!(
        "Layering partial parsing works! up_time: {:?}",
        effect.up_time
    );
}

#[test]
fn test_parse_layering_2lines() {
    let content = r#"show "Effect Layering Demo" {
    @00:00.000
    # Background layer: Static blue color with 2 second fade in
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace, up_time: 2s

    @00:02.000
    # Midground layer: Dimmer effect that slowly dims the blue with crossfades
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply, up_time: 1s, down_time: 1s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Effect Layering Demo").unwrap();
    assert_eq!(show.cues.len(), 2);
    println!("Layering 2 lines parsing works!");
}

#[test]
fn test_parse_layering_3lines() {
    let content = r#"show "Effect Layering Demo" {
    @00:00.000
    # Background layer: Static blue color with 2 second fade in
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace, up_time: 2s

    @00:02.000
    # Midground layer: Dimmer effect that slowly dims the blue with crossfades
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply, up_time: 1s, down_time: 1s

    @00:04.000
    # Foreground layer: Strobe effect on top of the dimmed blue with crossfades
    front_wash: strobe frequency: 2, layer: foreground, blend_mode: overlay, up_time: 0.5s, down_time: 0.5s, duration: 6s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Effect Layering Demo").unwrap();
    assert_eq!(show.cues.len(), 3);
    println!("Layering 3 lines parsing works!");
}

#[test]
fn test_parse_strobe_simple() {
    let content = r#"show "Test" {
    @00:00.000
    front_wash: strobe frequency: 2, up_time: 0.5s, down_time: 0.5s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_millis(500)));
    assert_eq!(effect.down_time, Some(Duration::from_millis(500)));
    println!(
        "Strobe simple parsing works! up_time: {:?}, down_time: {:?}",
        effect.up_time, effect.down_time
    );
}

#[test]
fn test_parse_strobe_no_crossfade() {
    let content = r#"show "Test" {
    @00:00.000
    front_wash: strobe frequency: 2
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, None);
    assert_eq!(effect.down_time, None);
    println!("Strobe no crossfade parsing works!");
}

#[test]
fn test_parse_strobe_crossfade_minimal() {
    let content = r#"show "Test" {
    @00:00.000
    front_wash: strobe frequency: 2, up_time: 0.5s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_millis(500)));
    assert_eq!(effect.down_time, None);
    println!(
        "Strobe timing minimal parsing works! up_time: {:?}",
        effect.up_time
    );
}

#[test]
fn test_parse_static_crossfade() {
    let content = r#"show "Test" {
    @00:00.000
    front_wash: static color: "blue", up_time: 0.5s, down_time: 0.5s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_millis(500)));
    assert_eq!(effect.down_time, Some(Duration::from_millis(500)));
    println!(
        "Static timing parsing works! up_time: {:?}, down_time: {:?}",
        effect.up_time, effect.down_time
    );
}

#[test]
fn test_parse_fade_in_only() {
    let content = r#"show "Test" {
    @00:00.000
    front_wash: static color: "blue", up_time: 0.5s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_millis(500)));
    assert_eq!(effect.down_time, None);
    println!("Up time only parsing works! up_time: {:?}", effect.up_time);
}

#[test]
fn test_parse_fade_in_simple() {
    let content = r#"show "Test" {
    @00:00.000
    front_wash: static color: "blue", up_time: 2s
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();
    let effect = &show.cues[0].effects[0];

    assert_eq!(effect.up_time, Some(Duration::from_secs(2)));
    assert_eq!(effect.down_time, None);
    println!(
        "Up time simple parsing works! up_time: {:?}",
        effect.up_time
    );
}

#[test]
fn test_inline_loop() {
    let content = r#"show "Test Show" {
    tempo {
        bpm: 120
        time_signature: 4/4
    }
    @0.0
    loop {
        @0.0
        effect: static, color: "red", duration: 1s
        @1.0
        effect: static, color: "blue", duration: 1s
    }, repeats: 3
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parser error: {}", e);
    }
    assert!(result.is_ok());
    let shows = result.unwrap();
    let show = &shows["Test Show"];

    // The inline loop with 3 repeats should create 6 cues (2 cues per iteration)
    // Each iteration is 2 seconds (1s red + 1s blue), so:
    // Iteration 0: @0.0 red, @1.0 blue
    // Iteration 1: @2.0 red, @3.0 blue
    // Iteration 2: @4.0 red, @5.0 blue
    assert!(
        show.cues.len() >= 6,
        "Expected at least 6 cues, got {}",
        show.cues.len()
    );

    // Check that we have cues at the expected times
    let cue_times: Vec<Duration> = show.cues.iter().map(|c| c.time).collect();
    println!("Cue times: {:?}", cue_times);
    println!("Number of cues: {}", show.cues.len());

    // Print details about each cue
    for (i, cue) in show.cues.iter().enumerate() {
        println!(
            "Cue {}: time={:?}, effects={}",
            i,
            cue.time,
            cue.effects.len()
        );
        for (j, effect) in cue.effects.iter().enumerate() {
            println!("  Effect {}: {:?}", j, effect.effect_type);
        }
    }

    // The loop should create 6 cues (2 cues per iteration × 3 iterations)
    // Expected times: 0.0, 1.0, 2.0, 3.0, 4.0, 5.0
    assert_eq!(
        show.cues.len(),
        6,
        "Expected 6 cues, got {}",
        show.cues.len()
    );

    // Check that we have cues at the expected times (allowing for small floating point differences)
    let expected_times = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let actual_times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();
    for expected in expected_times {
        assert!(
            actual_times.iter().any(|&t| (t - expected).abs() < 0.01),
            "Expected a cue at time {}, but got times: {:?}",
            expected,
            actual_times
        );
    }
}

#[test]
fn test_inline_loop_respects_base_offset() {
    // Loop starts at absolute @2.5 and should keep its internal relative timing.
    // Two cues inside the loop at 0.0s and 0.5s, loop duration 1.0s, repeats twice.
    // Expected cue times: 2.5, 3.0, 3.5, 4.0.
    let content = r#"show "Offset Loop" {
    tempo { bpm: 120 }
    @2.5
    loop {
        @0.0
        band: static, color: "red", duration: 0.5s
        @0.5
        band: static, color: "blue", duration: 0.5s
    }, repeats: 2
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["Offset Loop"];
    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();

    let expected = vec![2.5, 3.0, 3.5, 4.0];
    for t in expected {
        assert!(
            times.iter().any(|&actual| (actual - t).abs() < 0.01),
            "expected cue at {t}, got {times:?}"
        );
    }
    assert_eq!(times.len(), 4, "expected 4 cues, got {}", times.len());
}

#[test]
fn test_inline_loop_respects_tempo_for_durations() {
    // Tempo 120 BPM => 0.5s per beat.
    // One effect per loop iteration, duration 2 beats (1.0s).
    // Loop duration should be 1.0s; with repeats=2 we expect cues at 0.0 and 1.0.
    let content = r#"show "Tempo Loop" {
    tempo { bpm: 120 }
    @0.0
    loop {
        @0.0
        band: static, color: "red", duration: 2beats
    }, repeats: 2
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["Tempo Loop"];
    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();
    let expected_times = vec![0.0, 1.0];
    for t in expected_times {
        assert!(
            times.iter().any(|&actual| (actual - t).abs() < 0.01),
            "expected cue at {t}, got {times:?}"
        );
    }
    assert_eq!(times.len(), 2, "expected 2 cues, got {}", times.len());

    // Verify the duration parsed via tempo map (2 beats at 120 BPM = 1.0s)
    let dur = show.cues[0].effects[0]
        .total_duration()
        .expect("duration should be set");
    assert!(
        (dur.as_secs_f64() - 1.0).abs() < 0.01,
        "expected duration ~1.0s, got {:?}",
        dur
    );
}

#[test]
fn test_inline_loop_inside_sequence_preserves_times() {
    // Inline loop inside a sequence should expand to distinct cues with absolute times.
    // Two cues per loop iteration at 0.0s and 0.5s, loop duration 1.0s, repeats twice.
    // When the sequence is referenced at @0.0 the show should have cues at 0.0, 0.5, 1.0, 1.5.
    let content = r#"tempo { bpm: 120 }
sequence "LoopSeq" {
    @0.0
    loop {
        @0.0
        effect: static, color: "red", duration: 0.5s
        @0.5
        effect: static, color: "blue", duration: 0.5s
    }, repeats: 2
}

show "UseSeq" {
    @0.0
    sequence "LoopSeq"
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["UseSeq"];
    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();

    let expected = vec![0.0, 0.5, 1.0, 1.5];
    for t in expected {
        assert!(
            times.iter().any(|&actual| (actual - t).abs() < 0.01),
            "expected cue at {t}, got {times:?}"
        );
    }
    assert_eq!(times.len(), 4, "expected 4 cues, got {}", times.len());
}

#[test]
fn test_inline_loop_measure_time_positions() {
    // Measure-based timings inside an inline loop should be interpreted relative to the loop start.
    // Tempo 120 BPM, 4/4 => 1 measure = 2.0s, beat spacing = 0.5s.
    // Loop body cues at @1/1 (0.0s) and @1/3 (1.0s), each duration 1.0s; loop duration = 2.0s.
    // With repeats: 2, expected cue times: 0.0, 1.0, 2.0, 3.0.
    let content = r#"show "Measure Loop" {
    tempo {
        bpm: 120
        time_signature: 4/4
    }
    @0.0
    loop {
        @1/1
        effect: static, color: "red", duration: 1s
        @1/3
        effect: static, color: "blue", duration: 1s
    }, repeats: 2
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["Measure Loop"];
    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();

    let expected = vec![0.0, 1.0, 2.0, 3.0];
    for t in expected.iter() {
        assert!(
            times.iter().any(|&actual| (actual - t).abs() < 0.01),
            "expected cue at {t}, got {times:?}"
        );
    }
    assert_eq!(times.len(), 4, "expected 4 cues, got {}", times.len());
}

#[test]
fn test_base_effects_with_inline_loop_out_of_order() {
    // Test that base effects are scheduled at abs_time even when inline loop cues
    // are out of order (e.g., @0.5 appears before @0.0 in source)
    let content = r#"show "Base Effects Loop" {
    tempo { bpm: 120 }
    @1.0
    effect: static, color: "red", duration: 0.5s
    loop {
        @0.5
        effect: static, color: "blue", duration: 0.5s
        @0.0
        effect: static, color: "green", duration: 0.5s
    }, repeats: 1
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["Base Effects Loop"];

    // The base effect should be at @1.0 (abs_time)
    // The loop cues should be at @1.0 (green) and @1.5 (blue)
    // So we should have cues at: 1.0 (base red + loop green), 1.5 (loop blue)

    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();
    println!("Cue times: {:?}", times);

    // Find the cue at 1.0 - it should have both the base red effect and the loop green effect
    let cue_at_1_0 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.0).abs() < 0.01);
    assert!(cue_at_1_0.is_some(), "Expected a cue at time 1.0");

    let cue = cue_at_1_0.unwrap();
    // Should have at least 2 effects: base red + loop green
    assert!(
        cue.effects.len() >= 2,
        "Expected at least 2 effects at time 1.0 (base + loop), got {}",
        cue.effects.len()
    );

    // Verify we have a cue at 1.5 with the blue effect
    let cue_at_1_5 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 1.5).abs() < 0.01);
    assert!(cue_at_1_5.is_some(), "Expected a cue at time 1.5");

    // Verify the base effect is at the correct time (1.0), not at 1.5
    // The bug would put it at 1.5 if expanded_cues[0] was the first loop cue
    // We verify this by checking that the cue at 1.0 has multiple effects (base + loop)
    // and the cue at 1.5 only has the loop effect
    assert!(
        cue_at_1_5.unwrap().effects.len() == 1,
        "Cue at 1.5 should only have the loop blue effect, not the base red effect"
    );
}

#[test]
fn test_inline_loop_merging_consistent_between_show_and_sequence() {
    // Test that base effects and inline loop cues at the same time are merged consistently
    // in both shows and sequences. The sequence starts at @0.0 and is referenced at @0.0,
    // so the times should match exactly.
    let show_content = r#"show "Show Test" {
    tempo { bpm: 120 }
    @0.0
    effect: static, color: "red", duration: 0.5s
    loop {
        @0.0
        effect: static, color: "green", duration: 0.5s
    }, repeats: 1
}"#;

    let sequence_content = r#"tempo { bpm: 120 }
sequence "Seq Test" {
    @0.0
    effect: static, color: "red", duration: 0.5s
    loop {
        @0.0
        effect: static, color: "green", duration: 0.5s
    }, repeats: 1
}

show "Use Seq" {
    @0.0
    sequence "Seq Test"
}"#;

    // Parse show
    let shows = parse_light_shows(show_content).expect("show parse should succeed");
    let show = &shows["Show Test"];

    // Parse sequence
    let seq_shows = parse_light_shows(sequence_content).expect("sequence parse should succeed");
    let seq_show = &seq_shows["Use Seq"];

    // Both should have a cue at time 0.0 with 2 effects (base red + loop green merged)
    let show_cue_at_0_0 = show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 0.0).abs() < 0.01);
    let seq_cue_at_0_0 = seq_show
        .cues
        .iter()
        .find(|c| (c.time.as_secs_f64() - 0.0).abs() < 0.01);

    assert!(
        show_cue_at_0_0.is_some(),
        "Show should have a cue at time 0.0"
    );
    assert!(
        seq_cue_at_0_0.is_some(),
        "Sequence should have a cue at time 0.0"
    );

    let show_cue = show_cue_at_0_0.unwrap();
    let seq_cue = seq_cue_at_0_0.unwrap();

    // Both should have 2 effects (base red + loop green merged)
    assert_eq!(
        show_cue.effects.len(),
        2,
        "Show cue at 0.0 should have 2 effects (base + loop merged), got {}",
        show_cue.effects.len()
    );
    assert_eq!(
        seq_cue.effects.len(),
        2,
        "Sequence cue at 0.0 should have 2 effects (base + loop merged), got {}",
        seq_cue.effects.len()
    );

    // Verify both have the same number of cues total
    assert_eq!(
        show.cues.len(),
        seq_show.cues.len(),
        "Show and sequence should produce the same number of cues"
    );
}

#[test]
fn test_static_effect_duration_includes_up_and_down_time() {
    // Test that static effects with both duration and up_time/down_time calculate
    // total duration correctly (up_time + duration + down_time)
    let content = r#"show "Duration Test" {
    tempo { bpm: 120 }
    @0.0
    effect: static, color: "red", duration: 1s, up_time: 0.5s, down_time: 0.3s
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["Duration Test"];
    assert_eq!(show.cues.len(), 1, "Should have one cue");

    let effect = &show.cues[0].effects[0];
    let total_duration = effect
        .total_duration()
        .expect("effect should have a duration");

    // Total should be: 0.5s (up_time) + 1.0s (duration) + 0.3s (down_time) = 1.8s
    let expected_duration = Duration::from_secs_f64(1.8);
    assert!(
        (total_duration.as_secs_f64() - expected_duration.as_secs_f64()).abs() < 0.01,
        "Expected total duration ~1.8s, got {:?}",
        total_duration
    );
}

#[test]
fn test_inline_loop_with_static_duration_and_up_time() {
    // Test that inline loops correctly calculate duration when static effects have
    // both duration and up_time/down_time, preventing loop iterations from overlapping
    let content = r#"show "Loop Duration Test" {
    tempo { bpm: 120 }
    @0.0
    loop {
        @0.0
        effect: static, color: "red", duration: 1s, up_time: 0.5s, down_time: 0.3s
    }, repeats: 2
}"#;

    let shows = parse_light_shows(content).expect("parse should succeed");
    let show = &shows["Loop Duration Test"];

    // Loop duration should be 1.8s (0.5s up + 1.0s duration + 0.3s down)
    // With repeats: 2, we expect cues at 0.0 and 1.8
    let times: Vec<f64> = show.cues.iter().map(|c| c.time.as_secs_f64()).collect();
    let expected_times = vec![0.0, 1.8];

    for expected in expected_times {
        assert!(
            times.iter().any(|&actual| (actual - expected).abs() < 0.01),
            "Expected a cue at time {}, got times: {:?}",
            expected,
            times
        );
    }
    assert_eq!(
        times.len(),
        2,
        "Expected 2 cues (one per iteration), got {}",
        times.len()
    );
}
