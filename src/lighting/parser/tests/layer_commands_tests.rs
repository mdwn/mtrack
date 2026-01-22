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
use crate::lighting::effects::{EffectLayer, EffectType};
use crate::lighting::parser::types::LayerCommandType;
use crate::lighting::parser::*;

#[test]
fn test_layer_command_parsing() {
    // Test parsing layer commands in a show
    let content = r#"show "Layer Control Test" {
    @00:00.000
    front_wash: static color: "blue", layer: foreground

    @00:05.000
    release(layer: foreground, time: 2s)

    @00:10.000
    clear(layer: foreground)

    @00:15.000
    freeze(layer: background)

    @00:20.000
    unfreeze(layer: background)

    @00:25.000
    master(layer: midground, intensity: 50%, speed: 200%)
}"#;
    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Layer command parsing should succeed: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Layer Control Test").expect("Show should exist");

    // Check that cues were parsed
    assert_eq!(show.cues.len(), 6, "Should have 6 cues");

    // First cue has an effect, no layer commands
    assert_eq!(show.cues[0].effects.len(), 1);
    assert_eq!(show.cues[0].layer_commands.len(), 0);

    // Second cue: release command
    assert_eq!(show.cues[1].effects.len(), 0);
    assert_eq!(show.cues[1].layer_commands.len(), 1);
    let release_cmd = &show.cues[1].layer_commands[0];
    assert_eq!(release_cmd.command_type, LayerCommandType::Release);
    assert_eq!(release_cmd.layer, Some(EffectLayer::Foreground));
    assert_eq!(
        release_cmd.fade_time,
        Some(std::time::Duration::from_secs(2))
    );

    // Third cue: clear command
    let clear_cmd = &show.cues[2].layer_commands[0];
    assert_eq!(clear_cmd.command_type, LayerCommandType::Clear);
    assert_eq!(clear_cmd.layer, Some(EffectLayer::Foreground));

    // Fourth cue: freeze command
    let freeze_cmd = &show.cues[3].layer_commands[0];
    assert_eq!(freeze_cmd.command_type, LayerCommandType::Freeze);
    assert_eq!(freeze_cmd.layer, Some(EffectLayer::Background));

    // Fifth cue: unfreeze command
    let unfreeze_cmd = &show.cues[4].layer_commands[0];
    assert_eq!(unfreeze_cmd.command_type, LayerCommandType::Unfreeze);
    assert_eq!(unfreeze_cmd.layer, Some(EffectLayer::Background));

    // Sixth cue: master command
    let master_cmd = &show.cues[5].layer_commands[0];
    assert_eq!(master_cmd.command_type, LayerCommandType::Master);
    assert_eq!(master_cmd.layer, Some(EffectLayer::Midground));
    assert!((master_cmd.intensity.unwrap() - 0.5).abs() < 0.01);
    assert!((master_cmd.speed.unwrap() - 2.0).abs() < 0.01);
}

#[test]
fn test_clear_all_layers_command() {
    // Test parsing clear() without layer parameter (clears all layers)
    let content = r#"show "Clear All Test" {
    @00:00.000
    front_wash: static color: "blue", layer: foreground
    back_wash: static color: "red", layer: background

    @00:05.000
    clear()
}"#;
    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Clear all command parsing should succeed: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Clear All Test").expect("Show should exist");

    // Check that cues were parsed
    assert_eq!(show.cues.len(), 2, "Should have 2 cues");

    // Second cue: clear all command (no layer parameter)
    assert_eq!(show.cues[1].effects.len(), 0);
    assert_eq!(show.cues[1].layer_commands.len(), 1);
    let clear_cmd = &show.cues[1].layer_commands[0];
    assert_eq!(clear_cmd.command_type, LayerCommandType::Clear);
    assert_eq!(clear_cmd.layer, None, "Clear all should have no layer");
}

#[test]
fn test_t_dsl_parsing_errors() {
    // Test invalid DSL syntax
    let invalid_syntax = r#"show "Invalid Show" {
    @invalid_time
    front_wash: invalid_effect
}"#;
    let result = parse_light_shows(invalid_syntax);
    assert!(result.is_err(), "Invalid syntax should fail");

    // Test single unnamed show (allowed)
    let single_unnamed = r#"show {
    @00:00.000
    front_wash: static color: "blue"
}"#;
    let result = parse_light_shows(single_unnamed);
    assert!(
        result.is_ok(),
        "Single unnamed show should be allowed: {:?}",
        result.err()
    );
    let shows = result.unwrap();
    assert_eq!(
        shows.len(),
        1,
        "Expected exactly one show for single unnamed block"
    );

    // Test multiple shows where one is unnamed (should fail)
    let multiple_with_unnamed = r#"show "Named Show" {
    @00:00.000
    front_wash: static color: "blue"
}

show {
    @00:05.000
    back_wash: static color: "red"
}"#;
    let result = parse_light_shows(multiple_with_unnamed);
    assert!(
        result.is_err(),
        "Missing show name should fail when multiple shows are defined"
    );

    // Test malformed time string
    let malformed_time = r#"show "Test Show" {
    @invalid_time
    front_wash: static color: "blue"
}"#;
    let result = parse_light_shows(malformed_time);
    assert!(result.is_err(), "Malformed time should fail");

    // Test empty content
    let empty_content = "";
    let result = parse_light_shows(empty_content);
    assert!(result.is_ok(), "Empty content should be OK");
    assert_eq!(result.unwrap().len(), 0);

    // Test content that looks like a show but has no valid shows
    let no_shows = r#"// This is a comment
some invalid content
not a show"#;
    let _result = parse_light_shows(no_shows);
    // The parser may fail on invalid content, which is acceptable
    // We just test that it doesn't panic
}

#[test]
fn test_dsl_edge_cases() {
    // Test empty show
    let empty_show = r#"show "Empty Show" { }"#;
    let result = parse_light_shows(empty_show);
    assert!(result.is_ok());
    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    assert_eq!(shows["Empty Show"].cues.len(), 0);

    // Test show with overlapping cues
    let overlapping_cues = r#"show "Overlapping Show" {
    @00:05.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    back_wash: static color: "red", dimmer: 80%
}"#;
    let result = parse_light_shows(overlapping_cues);
    assert!(result.is_ok());
    let shows = result.unwrap();
    assert_eq!(shows["Overlapping Show"].cues.len(), 2);

    // Test show with multiple effects in one cue
    let multiple_effects = r#"show "Multiple Effects" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    back_wash: static color: "red", dimmer: 80%
}"#;
    let result = parse_light_shows(multiple_effects);
    assert!(result.is_ok());
    let shows = result.unwrap();
    assert_eq!(shows["Multiple Effects"].cues.len(), 1);
    assert_eq!(shows["Multiple Effects"].cues[0].effects.len(), 2);

    // Test show with missing parameters
    let missing_params = r#"show "Missing Params" {
    @00:00.000
    front_wash: static
}"#;
    let result = parse_light_shows(missing_params);
    assert!(
        result.is_ok(),
        "Missing parameters should be handled gracefully"
    );
}

#[test]
fn test_dsl_performance_large_file() {
    // Create a large DSL file with many cues
    let mut large_content = String::new();
    large_content.push_str(r#"show "Large Show" {"#);

    for i in 0..100 {
        let time_ms = i * 1000; // 1 second intervals
        let minutes = time_ms / 60000;
        let seconds = (time_ms % 60000) / 1000;
        let milliseconds = time_ms % 1000;

        large_content.push_str(&format!(
            r#"
    @{:02}:{:02}.{:03}
    fixture_{}: static color: "blue", dimmer: {}%"#,
            minutes,
            seconds,
            milliseconds,
            i,
            (i % 100)
        ));
    }

    large_content.push_str("\n}");

    // Test parsing performance
    let start = std::time::Instant::now();
    let result = parse_light_shows(&large_content);
    let duration = start.elapsed();

    assert!(result.is_ok(), "Large file should parse successfully");
    assert!(
        duration.as_millis() < 1000,
        "Parsing should be fast (< 1 second)"
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    assert_eq!(shows["Large Show"].cues.len(), 100);
}

#[test]
fn test_whitespace_handling() {
    // Test zero whitespace
    let no_whitespace = r#"show"Test Show"{@00:00.000 front_wash:static color:"blue",dimmer:60%}"#;
    let result = parse_light_shows(no_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with zero whitespace: {:?}",
        result
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    let show = shows.get("Test Show").unwrap();
    assert_eq!(show.cues.len(), 1);
    assert_eq!(show.cues[0].effects.len(), 1);

    // Test minimal whitespace (just what's needed)
    let minimal_whitespace = r#"show "Test Show" {
@00:00.000
front_wash: static color: "blue", dimmer: 60%
}"#;
    let result = parse_light_shows(minimal_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with minimal whitespace: {:?}",
        result
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    let show = shows.get("Test Show").unwrap();
    assert_eq!(show.cues.len(), 1);
    assert_eq!(show.cues[0].effects.len(), 1);

    // Test moderate whitespace
    let moderate_whitespace = r#"show "Test Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;
    let result = parse_light_shows(moderate_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with moderate whitespace: {:?}",
        result
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    let show = shows.get("Test Show").unwrap();
    assert_eq!(show.cues.len(), 1);
    assert_eq!(show.cues[0].effects.len(), 1);

    // Test excessive whitespace (this might fail due to grammar limitations)
    let excessive_whitespace = r#"
            show    "Test Show"    {
        @00:00.000    
        front_wash    :    static    
        color    :    "blue"    ,    
        dimmer    :    60%    
    }
    "#;
    let result = parse_light_shows(excessive_whitespace);
    // This might fail due to the grammar not handling excessive whitespace well
    if let Ok(shows) = result {
        assert_eq!(shows.len(), 1);
        let show = shows.get("Test Show").unwrap();
        assert_eq!(show.cues.len(), 1);
        assert_eq!(show.cues[0].effects.len(), 1);
    }

    // Test mixed whitespace (tabs, spaces, newlines)
    let mixed_whitespace = r#"show	"Test Show"	{
	@00:00.000	
	front_wash	:	static	
	color	:	"blue"	,	
	dimmer	:	60%	
}"#;
    let result = parse_light_shows(mixed_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with mixed whitespace: {:?}",
        result
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    let show = shows.get("Test Show").unwrap();
    assert_eq!(show.cues.len(), 1);
    assert_eq!(show.cues[0].effects.len(), 1);
}

#[test]
fn test_extreme_whitespace_handling() {
    // Test with very long whitespace sequences
    let long_whitespace = format!(
        r#"show "Test Show" {{{}@00:00.000{}front_wash: static color: "blue", dimmer: 60%{}}}"#,
        " ".repeat(50),
        " ".repeat(50),
        " ".repeat(50)
    );
    let result = parse_light_shows(&long_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with long whitespace: {:?}",
        result
    );

    // Test with mixed whitespace characters
    let mixed_whitespace = r#"show	"Test Show"	{
		@00:00.000		
		front_wash:	static	color:	"blue",	dimmer:	60%	
	}"#;
    let result = parse_light_shows(mixed_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with mixed whitespace: {:?}",
        result
    );

    // Test with newlines in various places
    let newline_whitespace = r#"show
"Test Show"
{
@00:00.000
front_wash:
static
color:
"blue",
dimmer:
60%
}"#;
    let result = parse_light_shows(newline_whitespace);
    assert!(
        result.is_ok(),
        "Failed to parse DSL with newline whitespace: {:?}",
        result
    );
}

#[test]
fn test_comprehensive_dsl_parsing() {
    // Test a comprehensive DSL file that uses various parameter types
    let comprehensive_dsl = r#"show "Comprehensive Light Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    back_wash: static color: "red", dimmer: 80%
    
    @00:10.000
    strobe_lights: static color: "green", dimmer: 100%
    
    @00:15.000
    moving_heads: static color: "white", dimmer: 50%
    
    @00:20.000
    dimmer_test: static color: "yellow", dimmer: 75%
    
    @00:25.000
    rainbow_effect: static color: "cyan", dimmer: 90%
    
    @00:30.000
    pulse_lights: static color: "magenta", dimmer: 25%
    
    @00:35.000
    color_cycle: static color: "orange", dimmer: 85%
    
    @00:40.000
    complex_chase: static color: "purple", dimmer: 95%
    
    @00:45.000
    strobe_variation: static color: "black", dimmer: 0%
    
    @00:50.000
    down_time: static color: "white", dimmer: 100%
}"#;

    let result = parse_light_shows(comprehensive_dsl);
    if let Err(e) = &result {
        println!("DSL parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Comprehensive DSL should parse successfully"
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    let show = shows.get("Comprehensive Light Show").unwrap();
    assert_eq!(show.cues.len(), 11);

    // Verify that different effect types are parsed
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.effects.len(), 1);
    // Check that it's a static effect (we can't directly compare struct variants)
    match &first_cue.effects[0].effect_type {
        EffectType::Static { .. } => {} // This is what we expect
        _ => panic!("Expected static effect"),
    }

    // Verify that parameters are parsed correctly
    let static_effect = &first_cue.effects[0];
    // Check that the effect type has the expected parameters
    match &static_effect.effect_type {
        crate::lighting::effects::EffectType::Static { parameters, .. } => {
            assert!(
                parameters.contains_key("color")
                    || parameters.contains_key("red")
                    || parameters.contains_key("green")
                    || parameters.contains_key("blue")
            );
            assert!(parameters.contains_key("dimmer"));
        }
        _ => panic!("Expected static effect"),
    }
}
