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
    let mut min_strobe_frequency = None;
    let mut strobe_dmx_offset = None;

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
                    &mut min_strobe_frequency,
                    &mut strobe_dmx_offset,
                )?;
            }
            _ => {}
        }
    }

    let mut fixture_type = FixtureType::new(name, channels);
    fixture_type.max_strobe_frequency = max_strobe_frequency;
    fixture_type.min_strobe_frequency = min_strobe_frequency;
    fixture_type.strobe_dmx_offset = strobe_dmx_offset;
    Ok(fixture_type)
}

fn parse_fixture_content(
    pair: Pair<Rule>,
    channels: &mut HashMap<String, u16>,
    special_cases: &mut Vec<String>,
    max_strobe_frequency: &mut Option<f64>,
    min_strobe_frequency: &mut Option<f64>,
    strobe_dmx_offset: &mut Option<u8>,
) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::channel_map => {
                *channels = parse_channel_mappings(content_pair);
            }
            Rule::max_strobe_frequency => {
                for inner in content_pair.into_inner() {
                    if inner.as_rule() == Rule::number_value {
                        let freq: f64 =
                            inner.as_str().trim().parse().map_err(|e| {
                                format!("Invalid max_strobe_frequency value: {}", e)
                            })?;
                        *max_strobe_frequency = Some(freq);
                    }
                }
            }
            Rule::min_strobe_frequency => {
                for inner in content_pair.into_inner() {
                    if inner.as_rule() == Rule::number_value {
                        let freq: f64 =
                            inner.as_str().trim().parse().map_err(|e| {
                                format!("Invalid min_strobe_frequency value: {}", e)
                            })?;
                        *min_strobe_frequency = Some(freq);
                    }
                }
            }
            Rule::strobe_dmx_offset => {
                for inner in content_pair.into_inner() {
                    if inner.as_rule() == Rule::number_value {
                        let offset: u8 = inner
                            .as_str()
                            .trim()
                            .parse()
                            .map_err(|e| format!("Invalid strobe_dmx_offset value: {}", e))?;
                        *strobe_dmx_offset = Some(offset);
                    }
                }
            }
            Rule::special_cases => {
                *special_cases = parse_special_case_list(content_pair);
            }
            _ => {}
        }
    }
    Ok(())
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
    let mut universe = 0u16;
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_fixture_types ──────────────────────────────────────

    #[test]
    fn fixture_type_basic_channels() {
        let content = r#"fixture_type "LED_Par" {
    channels: 4
    channel_map: {
        "red": 1,
        "green": 2,
        "blue": 3,
        "dimmer": 4
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);
        let ft = result.get("LED_Par").unwrap();
        assert_eq!(ft.name(), "LED_Par");
        assert_eq!(ft.channels().len(), 4);
        assert_eq!(ft.channels().get("red"), Some(&1));
        assert_eq!(ft.channels().get("dimmer"), Some(&4));
    }

    #[test]
    fn fixture_type_strobe_properties() {
        let content = r#"fixture_type "Strobe_Fix" {
    channels: 2
    channel_map: {
        "dimmer": 1,
        "strobe": 2
    }
    max_strobe_frequency: 25.0
    min_strobe_frequency: 0.5
    strobe_dmx_offset: 10
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Strobe_Fix").unwrap();
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.min_strobe_frequency(), Some(0.5));
        assert_eq!(ft.strobe_dmx_offset(), Some(10));
    }

    #[test]
    fn fixture_type_no_strobe() {
        let content = r#"fixture_type "Simple" {
    channels: 1
    channel_map: {
        "dimmer": 1
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Simple").unwrap();
        assert_eq!(ft.max_strobe_frequency(), None);
        assert_eq!(ft.min_strobe_frequency(), None);
        assert_eq!(ft.strobe_dmx_offset(), None);
    }

    #[test]
    fn fixture_type_multiple() {
        let content = r#"fixture_type "TypeA" {
    channels: 1
    channel_map: { "dimmer": 1 }
}

fixture_type "TypeB" {
    channels: 3
    channel_map: {
        "red": 1,
        "green": 2,
        "blue": 3
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("TypeA"));
        assert!(result.contains_key("TypeB"));
    }

    #[test]
    fn fixture_type_with_special_cases() {
        let content = r#"fixture_type "RGBW" {
    channels: 5
    channel_map: {
        "dimmer": 1,
        "red": 2,
        "green": 3,
        "blue": 4,
        "white": 5
    }
    special_cases: ["RGB", "Dimmer"]
}"#;
        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("RGBW"));
    }

    #[test]
    fn fixture_type_empty_input() {
        let result = parse_fixture_types("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn fixture_type_comments_only() {
        let content = "# This is a comment\n# Another comment\n";
        let result = parse_fixture_types(content).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn fixture_type_invalid_syntax() {
        let content = "fixture_type {";
        assert!(parse_fixture_types(content).is_err());
    }

    // ── parse_venues ─────────────────────────────────────────────

    #[test]
    fn venue_empty() {
        let content = r#"venue "Empty Hall" { }"#;
        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);
        let v = result.get("Empty Hall").unwrap();
        assert_eq!(v.name(), "Empty Hall");
        assert!(v.fixtures().is_empty());
        assert!(v.groups().is_empty());
    }

    #[test]
    fn venue_with_fixtures() {
        let content = r#"venue "Club" {
    fixture "Spot1" GenericPar @ 1:1
    fixture "Spot2" GenericPar @ 1:5
    fixture "Spot3" GenericPar @ 2:1
}"#;
        let result = parse_venues(content).unwrap();
        let v = result.get("Club").unwrap();
        assert_eq!(v.fixtures().len(), 3);

        let s1 = v.fixtures().get("Spot1").unwrap();
        assert_eq!(s1.fixture_type(), "GenericPar");
        assert_eq!(s1.universe(), 1);
        assert_eq!(s1.start_channel(), 1);

        let s3 = v.fixtures().get("Spot3").unwrap();
        assert_eq!(s3.universe(), 2);
        assert_eq!(s3.start_channel(), 1);
    }

    #[test]
    fn venue_with_tags() {
        let content = r#"venue "Tagged" {
    fixture "Wash1" Par @ 1:1 tags ["front", "wash"]
    fixture "Wash2" Par @ 1:5 tags ["back"]
}"#;
        let result = parse_venues(content).unwrap();
        let v = result.get("Tagged").unwrap();
        assert_eq!(v.fixtures().len(), 2);

        let w1 = v.fixtures().get("Wash1").unwrap();
        assert_eq!(w1.tags(), &["front", "wash"]);

        let w2 = v.fixtures().get("Wash2").unwrap();
        assert_eq!(w2.tags(), &["back"]);
    }

    #[test]
    fn venue_with_groups() {
        let content = r#"venue "Stage" {
    fixture "L1" Par @ 1:1
    fixture "L2" Par @ 1:5
    fixture "L3" Par @ 1:9
    group "front" = L1, L2, L3
}"#;
        let result = parse_venues(content).unwrap();
        let v = result.get("Stage").unwrap();
        assert_eq!(v.fixtures().len(), 3);
        assert_eq!(v.groups().len(), 1);

        let front = v.groups().get("front").unwrap();
        assert_eq!(front.name(), "front");
        assert_eq!(front.fixtures(), &["L1", "L2", "L3"]);
    }

    #[test]
    fn venue_multiple() {
        let content = r#"venue "A" {
    fixture "F1" Par @ 1:1
}

venue "B" {
    fixture "F2" Par @ 2:1
}"#;
        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains_key("A"));
        assert!(result.contains_key("B"));
    }

    #[test]
    fn venue_with_comments() {
        let content = r#"# Main venue
venue "Main" {
    fixture "F1" Par @ 1:1
}"#;
        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("Main"));
    }

    #[test]
    fn venue_empty_input() {
        let result = parse_venues("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn venue_invalid_syntax() {
        let content = "venue {";
        assert!(parse_venues(content).is_err());
    }

    // ── parse_fixture_definition (standalone) ────────────────────

    #[test]
    fn fixture_definition_basic() {
        let content = r#"fixture "MyLight" SomePar @ 3:17"#;
        let mut pairs = LightingParser::parse(Rule::fixture, content).unwrap();
        let pair = pairs.next().unwrap();
        let f = parse_fixture_definition(pair).unwrap();
        assert_eq!(f.name(), "MyLight");
        assert_eq!(f.fixture_type(), "SomePar");
        assert_eq!(f.universe(), 3);
        assert_eq!(f.start_channel(), 17);
    }

    #[test]
    fn fixture_definition_with_tags() {
        let content = r#"fixture "Spot" GenericSpot @ 1:100 tags ["front", "spot"]"#;
        let mut pairs = LightingParser::parse(Rule::fixture, content).unwrap();
        let pair = pairs.next().unwrap();
        let f = parse_fixture_definition(pair).unwrap();
        assert_eq!(f.name(), "Spot");
        assert_eq!(f.tags(), &["front", "spot"]);
    }
}
