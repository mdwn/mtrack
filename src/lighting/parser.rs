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

use super::types::{Fixture, FixtureType, Group, Venue};

#[derive(Parser)]
#[grammar = "src/lighting/grammar.pest"]
#[allow(dead_code)]
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

fn parse_fixture_type_definition(
    pair: pest::iterators::Pair<Rule>,
) -> Result<FixtureType, Box<dyn Error>> {
    let mut name = String::new();
    let mut channels = HashMap::new();
    let mut special_cases = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::string => {
                if name.is_empty() {
                    name = inner_pair.as_str().trim_matches('"').to_string();
                }
            }
            Rule::channel_definitions => {
                for channel_pair in inner_pair.into_inner() {
                    if let Rule::channel_definition = channel_pair.as_rule() {
                        let mut key = String::new();
                        let mut value = String::new();

                        for channel_inner in channel_pair.into_inner() {
                            match channel_inner.as_rule() {
                                Rule::identifier => {
                                    key = channel_inner.as_str().to_string();
                                }
                                Rule::string => {
                                    value = channel_inner.as_str().trim_matches('"').to_string();
                                }
                                Rule::number => {
                                    value = channel_inner.as_str().to_string();
                                }
                                Rule::special_case_list => {
                                    for special_case_pair in channel_inner.into_inner() {
                                        if let Rule::string = special_case_pair.as_rule() {
                                            special_cases.push(
                                                special_case_pair
                                                    .as_str()
                                                    .trim_matches('"')
                                                    .to_string(),
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        if key == "special_case" {
                            special_cases.push(value);
                        } else if key == "special_cases" {
                            // Already handled in the special_case_list rule
                        } else if !key.is_empty() && !value.is_empty() {
                            if let Ok(channel_num) = value.parse::<u16>() {
                                if channel_num == 0 {
                                    return Err(format!(
                                        "Channel number cannot be 0 for '{}'",
                                        key
                                    )
                                    .into());
                                }
                                channels.insert(key, channel_num);
                            } else {
                                return Err(format!(
                                    "Invalid channel number '{}' for '{}'",
                                    value, key
                                )
                                .into());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Fixture type name is required".into());
    }

    if channels.is_empty() {
        return Err(format!("Fixture type '{}' must have at least one channel", name).into());
    }

    Ok(FixtureType::new(name, channels, special_cases))
}

fn parse_venue_definition(pair: pest::iterators::Pair<Rule>) -> Result<Venue, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixtures = HashMap::new();
    let mut groups = HashMap::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::string => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::venue_content => {
                for content_pair in inner_pair.into_inner() {
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
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Venue name is required".into());
    }

    Ok(Venue::new(name, fixtures, groups))
}

fn parse_fixture_definition(pair: pest::iterators::Pair<Rule>) -> Result<Fixture, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixture_type = String::new();
    let mut universe = 0u32;
    let mut start_channel = 0u16;
    let mut tags: Vec<String> = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::string => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::identifier => {
                fixture_type = inner_pair.as_str().to_string();
            }
            Rule::number => {
                let num = inner_pair.as_str().parse::<u32>()?;
                if universe == 0 {
                    universe = num;
                } else {
                    start_channel = num as u16;
                }
            }
            Rule::tags => {
                for t in inner_pair.into_inner() {
                    // tag_list
                    for s in t.into_inner() {
                        // strings
                        if let Rule::string = s.as_rule() {
                            tags.push(s.as_str().trim_matches('"').to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Fixture name is required".into());
    }

    if fixture_type.is_empty() {
        return Err(format!("Fixture type is required for fixture '{}'", name).into());
    }

    if universe == 0 {
        return Err(format!("Universe must be greater than 0 for fixture '{}'", name).into());
    }

    if start_channel == 0 {
        return Err(format!(
            "Start channel must be greater than 0 for fixture '{}'",
            name
        )
        .into());
    }

    Ok(Fixture::new(
        name,
        fixture_type,
        universe,
        start_channel,
        tags,
    ))
}

fn parse_group_definition(pair: pest::iterators::Pair<Rule>) -> Result<Group, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixtures = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::string => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::identifier_list => {
                for fixture_pair in inner_pair.into_inner() {
                    if let Rule::identifier = fixture_pair.as_rule() {
                        fixtures.push(fixture_pair.as_str().to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Group name is required".into());
    }

    if fixtures.is_empty() {
        return Err(format!("Group '{}' must have at least one fixture", name).into());
    }

    Ok(Group::new(name, fixtures))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fixture_type() {
        let content = r#"type "RGBW Par" { red: 1, green: 2, blue: 3, white: 4, dimmer: 5, strobe: 6, special_case: "RGBW" }"#;

        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);

        let fixture_type = result.get("RGBW Par").unwrap();
        assert_eq!(fixture_type.name(), "RGBW Par");
        assert_eq!(fixture_type.channels().get("red"), Some(&1));
        assert_eq!(fixture_type.channels().get("green"), Some(&2));
        assert_eq!(fixture_type.channels().get("blue"), Some(&3));
        assert_eq!(fixture_type.channels().get("white"), Some(&4));
        assert_eq!(fixture_type.channels().get("dimmer"), Some(&5));
        assert_eq!(fixture_type.channels().get("strobe"), Some(&6));
        assert_eq!(fixture_type.special_cases(), &vec!["RGBW"]);
    }

    #[test]
    fn test_parse_fixture_type_multiple_special_cases() {
        let content = r#"type "Moving Head RGB" { pan: 1, tilt: 2, red: 3, green: 4, blue: 5, dimmer: 6, special_cases: "MovingHead", "RGB" }"#;

        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);

        let fixture_type = result.get("Moving Head RGB").unwrap();
        assert_eq!(fixture_type.name(), "Moving Head RGB");
        assert_eq!(fixture_type.channels().get("pan"), Some(&1));
        assert_eq!(fixture_type.channels().get("tilt"), Some(&2));
        assert_eq!(fixture_type.channels().get("red"), Some(&3));
        assert_eq!(fixture_type.channels().get("green"), Some(&4));
        assert_eq!(fixture_type.channels().get("blue"), Some(&5));
        assert_eq!(fixture_type.channels().get("dimmer"), Some(&6));
        assert_eq!(fixture_type.special_cases(), &vec!["MovingHead", "RGB"]);
    }

    #[test]
    fn test_parse_venue() {
        let content = r#"venue "Club Venue" { fixture "Wash1" RGBW_Par @ 1:1, fixture "Wash2" RGBW_Par @ 1:7, group "Front Wash" = Wash1, Wash2 }"#;

        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);

        let venue = result.get("Club Venue").unwrap();
        assert_eq!(venue.name(), "Club Venue");
        assert_eq!(venue.fixtures().len(), 2);
        assert_eq!(venue.groups().len(), 1);

        let wash1 = venue.fixtures().get("Wash1").unwrap();
        assert_eq!(wash1.name(), "Wash1");
        assert_eq!(wash1.fixture_type(), "RGBW_Par");
        assert_eq!(wash1.universe(), 1);
        assert_eq!(wash1.start_channel(), 1);

        let front_wash = venue.groups().get("Front Wash").unwrap();
        assert_eq!(front_wash.name(), "Front Wash");
        assert_eq!(front_wash.fixtures(), &vec!["Wash1", "Wash2"]);
    }

    #[test]
    fn test_parse_multiple_venues() {
        let content = r#"venue "Club Venue" { fixture "Wash1" RGBW_Par @ 1:1, group "Front Wash" = Wash1 } venue "Theater Venue" { fixture "Wash1" RGBW_Par @ 1:1, fixture "Wash2" RGBW_Par @ 1:7, group "Front Wash" = Wash1, Wash2 }"#;

        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 2);

        let club = result.get("Club Venue").unwrap();
        assert_eq!(club.fixtures().len(), 1);
        assert_eq!(club.groups().len(), 1);

        let theater = result.get("Theater Venue").unwrap();
        assert_eq!(theater.fixtures().len(), 2);
        assert_eq!(theater.groups().len(), 1);
    }
}
