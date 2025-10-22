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

use pest::Parser;
use pest_derive::Parser;
use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use super::types::{Fixture, FixtureType, Group, Venue};

#[derive(Parser)]
#[grammar = "src/lighting/grammar.pest"]
pub struct LightingParser;

pub fn parse_fixture_types(content: &str) -> Result<HashMap<String, FixtureType>, Box<dyn Error>> {
    let mut fixture_types = HashMap::new();

    let pairs = LightingParser::parse(Rule::file, content)
        .map_err(|e| format!("Failed to parse fixture types DSL: {}", e))?;

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::fixture_type => {
                    let fixture_type = parse_fixture_type_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse fixture type definition: {}", e))?;
                    fixture_types.insert(fixture_type.name().to_string(), fixture_type);
                }
                _ => {
                    // Skip non-fixture_type rules (like comments)
                }
            }
        }
    }

    Ok(fixture_types)
}

pub fn parse_venues(content: &str) -> Result<HashMap<String, Venue>, Box<dyn Error>> {
    let mut venues = HashMap::new();

    let pairs = LightingParser::parse(Rule::file, content)
        .map_err(|e| format!("Failed to parse venues DSL: {}", e))?;

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::venue => {
                    let venue = parse_venue_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse venue definition: {}", e))?;
                    venues.insert(venue.name().to_string(), venue);
                }
                _ => {
                    // Skip non-venue rules (like comments)
                }
            }
        }
    }

    Ok(venues)
}

// Light show DSL data structures
#[derive(Debug, Clone)]
pub struct LightShow {
    pub name: String,
    pub cues: Vec<Cue>,
}

#[derive(Debug, Clone)]
pub struct Cue {
    pub time: Duration,
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone)]
pub struct Effect {
    pub groups: Vec<String>,
    pub effect_type: EffectType,
    pub parameters: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum EffectType {
    Static,
    Cycle,
    Strobe,
    Pulse,
    Chase,
    Dimmer,
    Rainbow,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Color {
    #[allow(dead_code)]
    pub fn from_hex(hex: &str) -> Result<Self, Box<dyn Error>> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Err("Invalid hex color format".into());
        }

        let r = u8::from_str_radix(&hex[0..2], 16)?;
        let g = u8::from_str_radix(&hex[2..4], 16)?;
        let b = u8::from_str_radix(&hex[4..6], 16)?;

        Ok(Color {
            red: r,
            green: g,
            blue: b,
        })
    }

    #[allow(dead_code)]
    pub fn from_name(name: &str) -> Result<Self, Box<dyn Error>> {
        match name.to_lowercase().as_str() {
            "red" => Ok(Color {
                red: 255,
                green: 0,
                blue: 0,
            }),
            "green" => Ok(Color {
                red: 0,
                green: 255,
                blue: 0,
            }),
            "blue" => Ok(Color {
                red: 0,
                green: 0,
                blue: 255,
            }),
            "white" => Ok(Color {
                red: 255,
                green: 255,
                blue: 255,
            }),
            "black" => Ok(Color {
                red: 0,
                green: 0,
                blue: 0,
            }),
            "yellow" => Ok(Color {
                red: 255,
                green: 255,
                blue: 0,
            }),
            "cyan" => Ok(Color {
                red: 0,
                green: 255,
                blue: 255,
            }),
            "magenta" => Ok(Color {
                red: 255,
                green: 0,
                blue: 255,
            }),
            "orange" => Ok(Color {
                red: 255,
                green: 165,
                blue: 0,
            }),
            "purple" => Ok(Color {
                red: 128,
                green: 0,
                blue: 128,
            }),
            _ => Err(format!("Unknown color name: {}", name).into()),
        }
    }
}

/// Parses light shows from DSL content.
pub fn parse_light_shows(content: &str) -> Result<HashMap<String, LightShow>, Box<dyn Error>> {
    let pairs = LightingParser::parse(Rule::file, content)?;
    let mut shows = HashMap::new();

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::light_show {
                let show = parse_light_show_definition(inner_pair)?;
                shows.insert(show.name.clone(), show);
            }
        }
    }

    // If we have content that looks like a show but no shows were parsed, it's likely invalid syntax
    if shows.is_empty() && content.contains("show") {
        return Err("Failed to parse any shows - likely invalid syntax".into());
    }

    Ok(shows)
}

fn parse_light_show_definition(
    pair: pest::iterators::Pair<Rule>,
) -> Result<LightShow, Box<dyn Error>> {
    let mut name = String::new();
    let mut cues = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::show_name => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::show_content => {
                // Parse the show content which contains cues
                for content_pair in inner_pair.into_inner() {
                    if content_pair.as_rule() == Rule::cue {
                        let cue = parse_cue_definition(content_pair)?;
                        cues.push(cue);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(LightShow { name, cues })
}

fn parse_cue_definition(pair: pest::iterators::Pair<Rule>) -> Result<Cue, Box<dyn Error>> {
    let mut time = Duration::ZERO;
    let mut effects = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                time = parse_time_string(inner_pair.as_str())?;
            }
            Rule::effect => {
                let effect = parse_effect_definition(inner_pair)?;
                effects.push(effect);
            }
            Rule::comment => {
                // Skip comments
            }
            _ => {
                println!("Unexpected rule in cue: {:?}", inner_pair.as_rule());
            }
        }
    }

    Ok(Cue { time, effects })
}

#[allow(dead_code)]
fn parse_effect_definition(pair: pest::iterators::Pair<Rule>) -> Result<Effect, Box<dyn Error>> {
    let mut groups = Vec::new();
    let mut effect_type = EffectType::Static;
    let mut parameters = HashMap::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::group_list => {
                for group_pair in inner_pair.into_inner() {
                    if group_pair.as_rule() == Rule::group_name {
                        groups.push(group_pair.as_str().to_string());
                    }
                }
            }
            Rule::effect_type => {
                effect_type = match inner_pair.as_str() {
                    "static" => EffectType::Static,
                    "cycle" => EffectType::Cycle,
                    "strobe" => EffectType::Strobe,
                    "pulse" => EffectType::Pulse,
                    "chase" => EffectType::Chase,
                    "dimmer" => EffectType::Dimmer,
                    "rainbow" => EffectType::Rainbow,
                    _ => return Err(format!("Unknown effect type: {}", inner_pair.as_str()).into()),
                };
            }
            Rule::parameters => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::parameter {
                        let (key, value) = parse_parameter(param_pair)?;
                        parameters.insert(key, value);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(Effect {
        groups,
        effect_type,
        parameters,
    })
}

#[allow(dead_code)]
fn parse_time_string(time_str: &str) -> Result<Duration, Box<dyn Error>> {
    let time_str = time_str.trim_start_matches('@');
    let parts: Vec<&str> = time_str.split(':').collect();

    if parts.len() == 2 {
        // MM:SS.mmm format
        let minutes: u64 = parts[0].parse()?;
        let seconds_part = parts[1];
        let seconds_parts: Vec<&str> = seconds_part.split('.').collect();

        let seconds: u64 = seconds_parts[0].parse()?;
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse::<u64>()? * 10_u64.pow(3 - ms_str.len() as u32)
        } else {
            0
        };

        Ok(Duration::from_millis(
            minutes * 60 * 1000 + seconds * 1000 + milliseconds,
        ))
    } else {
        // SS.mmm format
        let seconds_parts: Vec<&str> = time_str.split('.').collect();
        let seconds: u64 = seconds_parts[0].parse()?;
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse::<u64>()? * 10_u64.pow(3 - ms_str.len() as u32)
        } else {
            0
        };

        Ok(Duration::from_millis(seconds * 1000 + milliseconds))
    }
}

#[allow(dead_code)]
fn parse_parameter(pair: pest::iterators::Pair<Rule>) -> Result<(String, String), Box<dyn Error>> {
    let mut key = String::new();
    let mut value = String::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::parameter_name => {
                key = inner_pair.as_str().to_string();
            }
            Rule::color_parameter => {
                value = parse_color_parameter(inner_pair)?;
            }
            Rule::dimmer_parameter => {
                value = parse_dimmer_parameter(inner_pair)?;
            }
            Rule::duration_parameter => {
                value = parse_duration_parameter(inner_pair)?;
            }
            Rule::fade_parameter => {
                value = parse_fade_parameter(inner_pair)?;
            }
            Rule::rate_parameter => {
                value = parse_rate_parameter(inner_pair)?;
            }
            Rule::duty_parameter => {
                value = parse_duty_parameter(inner_pair)?;
            }
            Rule::direction_parameter => {
                value = parse_direction_parameter(inner_pair)?;
            }
            Rule::loop_parameter => {
                value = parse_loop_parameter(inner_pair)?;
            }
            Rule::step_parameter => {
                value = parse_step_parameter(inner_pair)?;
            }
            Rule::transition_parameter => {
                value = parse_transition_parameter(inner_pair)?;
            }
            Rule::string_value => {
                value = parse_string_parameter(inner_pair)?;
            }
            Rule::number_value => {
                value = parse_number_parameter(inner_pair)?;
            }
            Rule::simple_value => {
                value = parse_simple_parameter(inner_pair)?;
            }
            _ => {
                value = inner_pair.as_str().to_string();
            }
        }
    }

    Ok((key, value))
}

#[allow(dead_code)]
fn parse_color_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_dimmer_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_duration_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_fade_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_rate_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_duty_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_direction_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_loop_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_step_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_transition_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_string_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_number_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

#[allow(dead_code)]
fn parse_simple_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

fn parse_fixture_type_definition(
    pair: pest::iterators::Pair<Rule>,
) -> Result<FixtureType, Box<dyn Error>> {
    let mut name = String::new();
    let mut channels = HashMap::new();
    let mut special_cases = Vec::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::fixture_type_name => {
                name = extract_string(pair);
            }
            Rule::fixture_type_content => {
                parse_fixture_content(pair, &mut channels, &mut special_cases);
            }
            _ => {}
        }
    }

    Ok(FixtureType::new(name, channels, special_cases))
}

fn parse_fixture_content(
    pair: pest::iterators::Pair<Rule>,
    channels: &mut HashMap<String, u16>,
    special_cases: &mut Vec<String>,
) {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::channel_map => {
                *channels = parse_channel_mappings(content_pair);
            }
            Rule::special_cases => {
                *special_cases = parse_special_case_list(content_pair);
            }
            _ => {}
        }
    }
}

fn parse_channel_mappings(pair: pest::iterators::Pair<Rule>) -> HashMap<String, u16> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::channel_mapping_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::channel_mapping)
        .filter_map(|mapping| {
            let mut key = String::new();
            let mut value = 0u16;

            for inner in mapping.into_inner() {
                match inner.as_rule() {
                    Rule::channel_name => key = extract_string(inner),
                    Rule::channel_number => value = inner.as_str().trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
            if !key.is_empty() && value > 0 {
                Some((key, value))
            } else {
                None
            }
        })
        .collect()
}

fn parse_special_case_list(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::special_case_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::special_case)
        .map(|case| extract_string(case))
        .collect()
}

fn extract_string(pair: pest::iterators::Pair<Rule>) -> String {
    pair.as_str().trim_matches('"').to_string()
}

fn parse_venue_definition(pair: pest::iterators::Pair<Rule>) -> Result<Venue, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixtures = HashMap::new();
    let mut groups = HashMap::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::venue_content => {
                parse_venue_content(pair, &mut fixtures, &mut groups)?;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Venue name is required".into());
    }

    Ok(Venue::new(name, fixtures, groups))
}

fn parse_venue_content(
    pair: pest::iterators::Pair<Rule>,
    fixtures: &mut HashMap<String, Fixture>,
    groups: &mut HashMap<String, Group>,
) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::fixture => {
                let fixture = parse_fixture_definition(content_pair)?;
                fixtures.insert(fixture.name().to_string(), fixture);
            }
            Rule::group => {
                let group = parse_group_definition(content_pair)?;
                groups.insert(group.name().to_string(), group);
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_fixture_definition(pair: pest::iterators::Pair<Rule>) -> Result<Fixture, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixture_type = String::new();
    let mut universe = 0u32;
    let mut start_channel = 0u16;
    let mut tags = Vec::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::identifier => {
                fixture_type = pair.as_str().to_string();
            }
            Rule::number => {
                let value: u32 = pair.as_str().parse()?;
                if universe == 0 {
                    universe = value;
                } else {
                    start_channel = value as u16;
                }
            }
            Rule::tags => {
                tags = parse_tags(pair);
            }
            _ => {}
        }
    }

    Ok(Fixture::new(
        name,
        fixture_type,
        universe,
        start_channel,
        tags,
    ))
}

fn parse_tags(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::string)
        .map(|tag| extract_string(tag))
        .collect()
}

fn parse_group_definition(pair: pest::iterators::Pair<Rule>) -> Result<Group, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixtures = Vec::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::identifier_list => {
                fixtures = parse_identifier_list(pair);
            }
            _ => {}
        }
    }

    Ok(Group::new(name, fixtures))
}

fn parse_identifier_list(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::identifier)
        .map(|id| id.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_light_show() {
        let content = r#"show "Test Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Test Show"));
    }

    #[test]
    fn test_parse_basic_show() {
        let content = r#"show "Basic Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Basic Show"));
    }

    #[test]
    fn test_parse_complex_show() {
        let content = r#"show "Complex Show" {
    @00:00.000
    front_wash: static color: "red", dimmer: 80%
    
    @00:05.000
    back_wash: static color: "blue", dimmer: 60%
    
    @00:10.000
    movers: static color: "green", dimmer: 100%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Complex Show"));

        let show = &shows["Complex Show"];
        println!("Found {} cues", show.cues.len());
        assert_eq!(show.cues.len(), 3);
    }

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
        assert_eq!(red.red, 255);
        assert_eq!(red.green, 0);
        assert_eq!(red.blue, 0);

        // Test named colors
        let blue = Color::from_name("blue").unwrap();
        assert_eq!(blue.red, 0);
        assert_eq!(blue.green, 0);
        assert_eq!(blue.blue, 255);

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
    fn test_parse_simple_example() {
        let content = r#"show "Simple Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Simple Show"));

        let show = &shows["Simple Show"];
        println!("Found {} cues in simple example", show.cues.len());
        assert_eq!(show.cues.len(), 1);
    }

    #[test]
    fn test_parse_advanced_example() {
        let content = r#"show "Advanced Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:04.000
    back_wash: static color: "orange", dimmer: 50%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Advanced Show"));

        let show = &shows["Advanced Show"];
        println!("Found {} cues in advanced example", show.cues.len());
        assert_eq!(show.cues.len(), 2);
    }

    #[test]
    fn test_parse_cycle_effect() {
        let content = r#"show "Cycle Show" {
    @00:00.000
    front_wash: cycle color: "red", color: "green", color: "blue", duration: 3s, direction: forward, dimmer: 50%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Cycle Show"));

        let show = &shows["Cycle Show"];
        println!("Found {} cues in cycle example", show.cues.len());
        assert_eq!(show.cues.len(), 1);
    }

    #[test]
    fn test_parse_chase_effect() {
        let content = r#"show "Chase Show" {
    @00:00.000
    movers: chase steps: [
        {color: "purple", dimmer: 100%, duration: 400ms, transition: fade},
        {color: "cyan", dimmer: 70%, duration: 400ms, transition: fade}
    ], loop: pingpong, direction: forward
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Chase Show"));

        let show = &shows["Chase Show"];
        println!("Found {} cues in chase example", show.cues.len());
        assert_eq!(show.cues.len(), 1);
    }

    #[test]
    fn test_parse_full_example() {
        let content = r#"show "Full Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:04.000
    back_wash: static color: "orange", dimmer: 50%, fade: 500ms
    
    @00:16.000
    front_wash, back_wash: cycle color: "red", color: "green", color: "blue", duration: 3s, direction: forward, dimmer: 50%
    
    @00:30.000
    movers: chase steps: [
        {color: "purple", dimmer: 100%, duration: 400ms, transition: fade},
        {color: "cyan", dimmer: 70%, duration: 400ms, transition: fade}
    ], loop: pingpong, direction: forward
    
    @01:00.000
    strobe: strobe rate: 8, duty: 20%, duration: 2s, dimmer: 80%
    
    @01:05.000
    front_wash, back_wash, movers: dimmer dimmer: 0%, fade: 1000ms
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Full Show"));

        let show = &shows["Full Show"];
        println!("Found {} cues in full example", show.cues.len());
        assert_eq!(show.cues.len(), 6);
    }

    #[test]
    fn test_parse_fixture_type() {
        let content = r#"fixture_type "RGBW_Par" {
            channels: 4
            channel_map: {
                "dimmer": 1,
                "red": 2,
                "green": 3,
                "blue": 4
            }
            special_cases: ["RGB", "Dimmer"]
        }"#;

        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);

        let fixture_type = result.get("RGBW_Par").unwrap();
        assert_eq!(fixture_type.name(), "RGBW_Par");
        assert_eq!(fixture_type.channels().get("dimmer"), Some(&1));
        assert_eq!(fixture_type.channels().get("red"), Some(&2));
        assert_eq!(fixture_type.channels().get("green"), Some(&3));
        assert_eq!(fixture_type.channels().get("blue"), Some(&4));
        assert_eq!(fixture_type.special_cases(), &vec!["RGB", "Dimmer"]);
    }

    #[test]
    fn test_parse_venue() {
        let content = r#"venue "Club Venue" { }"#;

        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);

        let venue = result.get("Club Venue").unwrap();
        assert_eq!(venue.name(), "Club Venue");
        assert_eq!(venue.fixtures().len(), 0);
        assert_eq!(venue.groups().len(), 0);
    }
}
