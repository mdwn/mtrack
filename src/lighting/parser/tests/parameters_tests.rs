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
use crate::lighting::effects::{Color, EffectType};
use crate::lighting::parser::*;

#[test]
fn test_parameter_population() {
    // Test that parameters are properly populated in effect types
    let simple_dsl = r#"show "Parameter Test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    back_wash: static color: "red", dimmer: 80%
}"#;

    let result = parse_light_shows(simple_dsl);
    assert!(result.is_ok(), "Simple DSL should parse successfully");

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);

    let show = shows.get("Parameter Test").unwrap();
    assert_eq!(show.cues.len(), 2);

    // Test first cue
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.effects.len(), 1);

    let first_effect = &first_cue.effects[0];
    assert_eq!(first_effect.groups, vec!["front_wash"]);

    // Check that parameters are stored in the EffectType
    match &first_effect.effect_type {
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

    // Check that effect type is properly populated
    match &first_effect.effect_type {
        EffectType::Static {
            parameters: static_params,
            duration,
        } => {
            // Check that parameters were applied to the effect type
            assert!(static_params.contains_key("red"));
            assert!(static_params.contains_key("green"));
            assert!(static_params.contains_key("blue"));
            assert!(static_params.contains_key("dimmer"));

            // Check specific values
            assert_eq!(static_params.get("dimmer"), Some(&0.6)); // 60% converted to 0.6
            assert_eq!(static_params.get("blue"), Some(&1.0)); // Blue color should be 1.0
            assert_eq!(static_params.get("red"), Some(&0.0)); // Red should be 0.0
            assert_eq!(static_params.get("green"), Some(&0.0)); // Green should be 0.0

            // Duration should be None for static effects without duration parameter
            assert_eq!(*duration, None);
        }
        _ => panic!("Expected static effect type"),
    }

    // Test second cue
    let second_cue = &show.cues[1];
    let second_effect = &second_cue.effects[0];
    assert_eq!(second_effect.groups, vec!["back_wash"]);
    // Check that the second effect has the expected parameters
    match &second_effect.effect_type {
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

    // Check that red effect was properly applied
    match &second_effect.effect_type {
        EffectType::Static {
            parameters: static_params,
            ..
        } => {
            assert_eq!(static_params.get("dimmer"), Some(&0.8)); // 80% converted to 0.8
            assert_eq!(static_params.get("red"), Some(&1.0)); // Red should be 1.0
            assert_eq!(static_params.get("blue"), Some(&0.0)); // Blue should be 0.0
            assert_eq!(static_params.get("green"), Some(&0.0)); // Green should be 0.0
        }
        _ => panic!("Expected static effect type"),
    }
}

#[test]
fn test_t_advanced_parameter_parsing() {
    // Test DSL that should use advanced parameter parsing functions
    // Using the exact syntax the grammar expects
    let advanced_dsl = r#"show "Advanced Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%, fade: 2s
    
    @00:05.000
    back_wash: cycle speed: 1.5, direction: forward
    
    @00:10.000
    strobe_lights: strobe frequency: 8, intensity: 0.8, duration: 5s
    
    @00:15.000
    moving_heads: chase loop: pingpong, direction: random, transition: crossfade
    
    @00:20.000
    dimmer_test: dimmer start: 0%, end: 100%, duration: 3s, curve: linear
    
    @00:25.000
    rainbow_effect: rainbow speed: 2.0, direction: forward
    
    @00:30.000
    pulse_lights: pulse frequency: 4, intensity: 0.6, duty: 50%
}"#;

    let result = parse_light_shows(advanced_dsl);
    if let Err(e) = &result {
        println!("Advanced DSL parsing error: {}", e);
    }
    // This might fail if the grammar doesn't support the advanced syntax
    // but it should help us understand what's actually supported
    println!("Advanced DSL parsing result: {:?}", result);
}

#[test]
fn test_simple_advanced_parameters() {
    // Test with just one advanced parameter at a time to isolate issues
    let simple_advanced = r#"show "Simple Advanced" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%, fade: 2s
}"#;

    let result = parse_light_shows(simple_advanced);
    if let Err(e) = &result {
        println!("Simple advanced DSL parsing error: {}", e);
    }
    println!("Simple advanced DSL parsing result: {:?}", result);
}

#[test]
fn test_custom_color_formats() {
    // Test all three supported color formats
    let custom_colors_dsl = r##"show "Custom Colors Show" {
    @00:00.000
    front_wash: static color: "#ff0000", dimmer: 60%
    
    @00:05.000
    back_wash: static color: rgb(0, 255, 0), dimmer: 80%
    
    @00:10.000
    side_wash: static color: "purple", dimmer: 100%
}"##;

    let result = parse_light_shows(custom_colors_dsl);
    if let Err(e) = &result {
        println!("Custom colors DSL parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Custom colors DSL should parse successfully"
    );

    let shows = result.unwrap();
    let show = shows.get("Custom Colors Show").unwrap();
    assert_eq!(show.cues.len(), 3);

    // Verify that colors are parsed correctly in the effect types
    let first_cue = &show.cues[0];
    match &first_cue.effects[0].effect_type {
        crate::lighting::effects::EffectType::Static { parameters, .. } => {
            assert!(
                parameters.contains_key("color")
                    || parameters.contains_key("red")
                    || parameters.contains_key("green")
                    || parameters.contains_key("blue")
            );
        }
        _ => panic!("Expected static effect"),
    }

    let second_cue = &show.cues[1];
    match &second_cue.effects[0].effect_type {
        crate::lighting::effects::EffectType::Static { parameters, .. } => {
            assert!(
                parameters.contains_key("color")
                    || parameters.contains_key("red")
                    || parameters.contains_key("green")
                    || parameters.contains_key("blue")
            );
        }
        _ => panic!("Expected static effect"),
    }

    let third_cue = &show.cues[2];
    match &third_cue.effects[0].effect_type {
        crate::lighting::effects::EffectType::Static { parameters, .. } => {
            assert!(
                parameters.contains_key("color")
                    || parameters.contains_key("red")
                    || parameters.contains_key("green")
                    || parameters.contains_key("blue")
            );
        }
        _ => panic!("Expected static effect"),
    }
}

#[test]
fn test_t_user_dsl_syntax() {
    let content = r#"show "Shieldbrother" {
@00:00.000
front_wash: static, color: "blue"

@00:05.000
all_wash: cycle, color: "red", color: "green", color: "blue", speed: 1.5, direction: "forward"
}"#;

    let result = parse_light_shows(content);
    if let Err(e) = &result {
        println!("Parse error: {}", e);
    }
    assert!(result.is_ok());
    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);
    assert!(shows.contains_key("Shieldbrother"));

    let show = shows.get("Shieldbrother").unwrap();
    assert_eq!(show.cues.len(), 2);

    // Check first cue (static effect)
    let first_cue = &show.cues[0];
    assert_eq!(first_cue.time.as_nanos(), 0);
    assert_eq!(first_cue.effects.len(), 1);
    assert_eq!(first_cue.effects[0].groups, vec!["front_wash"]);

    // Check second cue (cycle effect)
    let second_cue = &show.cues[1];
    assert_eq!(second_cue.time.as_secs(), 5);
    assert_eq!(second_cue.effects.len(), 1);
    assert_eq!(second_cue.effects[0].groups, vec!["all_wash"]);

    // Verify that the cycle effect has multiple colors
    if let EffectType::ColorCycle {
        colors,
        speed: _,
        direction: _,
        transition: _,
    } = &second_cue.effects[0].effect_type
    {
        assert_eq!(colors.len(), 3, "Cycle effect should have 3 colors");
        // Check that the colors are correct
        assert_eq!(
            colors[0],
            Color::new(255, 0, 0),
            "First color should be red"
        );
        assert_eq!(
            colors[1],
            Color::new(0, 255, 0),
            "Second color should be green"
        );
        assert_eq!(
            colors[2],
            Color::new(0, 0, 255),
            "Third color should be blue"
        );
    } else {
        panic!("Expected ColorCycle effect type");
    }
}
