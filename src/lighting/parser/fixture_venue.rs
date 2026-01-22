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

use std::collections::HashMap;
use std::error::Error;

use super::super::types::{Fixture, FixtureType, Group, Venue};
use super::error::get_error_context;
use super::grammar::{LightingParser, Rule};
use pest::iterators::Pair;
use pest::Parser;

pub fn parse_fixture_types(content: &str) -> Result<HashMap<String, FixtureType>, Box<dyn Error>> {
    let mut fixture_types = HashMap::new();

    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "Fixture types DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            ).into());
        }
    };

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

    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "Venues DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            )
            .into());
        }
    };

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::venue => {
                    let venue = parse_venue_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse venue definition: {}", e))?;
                    venues.insert(venue.name().to_string(), venue);
                }
                _ => {
                    // Ignore other rules
                }
            }
        }
    }

    Ok(venues)
}

fn parse_fixture_type_definition(pair: Pair<Rule>) -> Result<FixtureType, Box<dyn Error>> {
    let mut name = String::new();
    let mut channels = HashMap::new();
    let mut special_cases = Vec::new();
    let mut max_strobe_frequency = None;

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::fixture_type_name => {
                name = extract_string(pair);
            }
            Rule::fixture_type_content => {
                parse_fixture_content(
                    pair,
                    &mut channels,
                    &mut special_cases,
                    &mut max_strobe_frequency,
                );
            }
            _ => {}
        }
    }

    let mut fixture_type = FixtureType::new(name, channels, special_cases);
    fixture_type.max_strobe_frequency = max_strobe_frequency;
    Ok(fixture_type)
}

fn parse_fixture_content(
    pair: Pair<Rule>,
    channels: &mut HashMap<String, u16>,
    special_cases: &mut Vec<String>,
    max_strobe_frequency: &mut Option<f64>,
) {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::channel_map => {
                *channels = parse_channel_mappings(content_pair);
            }
            Rule::max_strobe_frequency => {
                *max_strobe_frequency = Some(content_pair.as_str().trim().parse().unwrap_or(0.0));
            }
            Rule::special_cases => {
                *special_cases = parse_special_case_list(content_pair);
            }
            _ => {}
        }
    }
}

fn parse_channel_mappings(pair: Pair<Rule>) -> HashMap<String, u16> {
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

fn parse_special_case_list(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::special_case_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::special_case)
        .map(|case| extract_string(case))
        .collect()
}

fn extract_string(pair: Pair<Rule>) -> String {
    pair.as_str().trim_matches('"').trim().to_string()
}

fn parse_venue_definition(pair: Pair<Rule>) -> Result<Venue, Box<dyn Error>> {
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
    pair: Pair<Rule>,
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

pub(crate) fn parse_fixture_definition(pair: Pair<Rule>) -> Result<Fixture, Box<dyn Error>> {
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
            Rule::universe_num => {
                universe = pair.as_str().trim().parse()?;
            }
            Rule::address_num => {
                start_channel = pair.as_str().trim().parse()?;
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

fn parse_tags(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::tag_list)
        .flat_map(|tag_list| {
            tag_list
                .into_inner()
                .filter(|p| p.as_rule() == Rule::string)
                .map(|tag| extract_string(tag))
        })
        .collect()
}

fn parse_group_definition(pair: Pair<Rule>) -> Result<Group, Box<dyn Error>> {
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

fn parse_identifier_list(pair: Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::identifier)
        .map(|id| id.as_str().trim().to_string())
        .collect()
}
