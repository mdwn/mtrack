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
