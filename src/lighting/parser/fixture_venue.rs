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

use super::super::types::{
    ChannelDef, ChannelFunction, Fixture, FixtureType, GdtfSource, MovementLimits, PhysicalRange,
    PhysicalUnit, Vec3, Venue,
};
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
    let mut channel_defs: HashMap<String, ChannelDef> = HashMap::new();
    let mut special_cases = Vec::new();
    let mut max_strobe_frequency = None;
    let mut min_strobe_frequency = None;
    let mut strobe_dmx_offset = None;
    let mut source = None;
    let mut movement = MovementLimits::default();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::fixture_type_name => {
                name = extract_string(pair);
            }
            Rule::gdtf_source => {
                source = Some(parse_gdtf_source(pair)?);
            }
            Rule::fixture_type_content => {
                parse_fixture_content(
                    pair,
                    &mut channel_defs,
                    &mut movement,
                    &mut special_cases,
                    &mut max_strobe_frequency,
                    &mut min_strobe_frequency,
                    &mut strobe_dmx_offset,
                )?;
            }
            _ => {}
        }
    }

    let mut fixture_type = FixtureType::from_channel_defs(name, channel_defs);
    // v1 strobe fields, when present, take effect on top of anything the
    // structured channels derived; a v1-only file then gets its function
    // synthesized so the structured view carries the same information.
    if max_strobe_frequency.is_some() {
        fixture_type.max_strobe_frequency = max_strobe_frequency;
    }
    if min_strobe_frequency.is_some() {
        fixture_type.min_strobe_frequency = min_strobe_frequency;
    }
    if strobe_dmx_offset.is_some() {
        fixture_type.strobe_dmx_offset = strobe_dmx_offset;
    }
    fixture_type.normalize_legacy_strobe();
    if let Some(source) = source {
        fixture_type.set_source(source);
    }
    fixture_type.set_movement(movement);
    Ok(fixture_type)
}

fn parse_gdtf_source(pair: Pair<Rule>) -> Result<GdtfSource, Box<dyn Error>> {
    let mut strings = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::string)
        .map(extract_string);
    let path = strings.next().ok_or("gdtf source requires a path")?;
    let mode = strings.next().ok_or("gdtf source requires a mode")?;
    Ok(GdtfSource { path, mode })
}

fn parse_fixture_content(
    pair: Pair<Rule>,
    channel_defs: &mut HashMap<String, ChannelDef>,
    movement: &mut MovementLimits,
    special_cases: &mut Vec<String>,
    max_strobe_frequency: &mut Option<f64>,
    min_strobe_frequency: &mut Option<f64>,
    strobe_dmx_offset: &mut Option<u8>,
) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::channel_map => {
                for (name, offset) in parse_channel_mappings(content_pair) {
                    channel_defs.insert(name, ChannelDef::at(offset));
                }
            }
            Rule::channel_def => {
                let (name, def) = parse_channel_def(content_pair)?;
                channel_defs.insert(name, def);
            }
            Rule::movement_block => {
                *movement = parse_movement_block(content_pair)?;
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

fn parse_channel_def(pair: Pair<Rule>) -> Result<(String, ChannelDef), Box<dyn Error>> {
    let mut name = String::new();
    let mut def = ChannelDef::at(0);

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => name = extract_string(inner),
            Rule::number_value => {
                def.offset = inner
                    .as_str()
                    .trim()
                    .parse()
                    .map_err(|e| format!("Invalid channel offset: {e}"))?;
            }
            Rule::channel_fine => {
                let value = inner
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::number_value)
                    .ok_or("fine requires an offset")?;
                def.fine = Some(
                    value
                        .as_str()
                        .trim()
                        .parse()
                        .map_err(|e| format!("Invalid fine offset: {e}"))?,
                );
            }
            Rule::channel_range => {
                def.range = Some(parse_channel_range(inner)?);
            }
            Rule::channel_block => {
                for block_pair in inner.into_inner() {
                    match block_pair.as_rule() {
                        Rule::channel_functions => {
                            for entry in block_pair
                                .into_inner()
                                .filter(|p| p.as_rule() == Rule::function_entry)
                            {
                                def.functions.push(parse_function_entry(entry)?);
                            }
                        }
                        Rule::channel_range => {
                            def.range = Some(parse_channel_range(block_pair)?);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("channel requires a name".into());
    }
    if def.offset == 0 {
        return Err(format!("channel \"{name}\" requires a 1-based offset").into());
    }
    Ok((name, def))
}

fn parse_channel_range(pair: Pair<Rule>) -> Result<PhysicalRange, Box<dyn Error>> {
    let range = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::phys_range)
        .ok_or("range requires physical values")?;
    parse_phys_range(range)
}

fn parse_function_entry(pair: Pair<Rule>) -> Result<ChannelFunction, Box<dyn Error>> {
    let mut name = String::new();
    let mut dmx_from = 0u8;
    let mut dmx_to = 0u8;
    let mut physical = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::string => name = extract_string(inner),
            Rule::dmx_range => {
                let mut numbers = inner
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::number_value);
                let from = numbers.next().ok_or("dmx range requires a start")?;
                let to = numbers.next().ok_or("dmx range requires an end")?;
                dmx_from = from
                    .as_str()
                    .trim()
                    .parse()
                    .map_err(|e| format!("Invalid DMX range start: {e}"))?;
                dmx_to = to
                    .as_str()
                    .trim()
                    .parse()
                    .map_err(|e| format!("Invalid DMX range end: {e}"))?;
            }
            Rule::phys_range => {
                physical = Some(parse_phys_range(inner)?);
            }
            _ => {}
        }
    }

    if dmx_from > dmx_to {
        return Err(format!(
            "function \"{name}\": DMX range {dmx_from}..{dmx_to} is inverted"
        )
        .into());
    }
    Ok(ChannelFunction {
        name,
        dmx_from,
        dmx_to,
        physical,
    })
}

fn parse_phys_range(pair: Pair<Rule>) -> Result<PhysicalRange, Box<dyn Error>> {
    let mut values = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::phys_value)
        .map(|p| parse_phys_value(p.as_str()));
    let (from, from_unit) = values.next().ok_or("physical range requires a start")??;
    let (to, to_unit) = values.next().ok_or("physical range requires an end")??;
    if from_unit != to_unit {
        return Err(format!(
            "physical range mixes units: {}..{}",
            from_unit.suffix(),
            to_unit.suffix()
        )
        .into());
    }
    Ok(PhysicalRange {
        from,
        to,
        unit: from_unit,
    })
}

/// Splits an atomic physical value like "-270deg" or "0.3hz" back into value
/// and unit.
fn parse_phys_value(text: &str) -> Result<(f64, PhysicalUnit), Box<dyn Error>> {
    let text = text.trim();
    let (number, unit) = if let Some(number) = text.strip_suffix("deg") {
        (number, PhysicalUnit::Degrees)
    } else if let Some(number) = text.strip_suffix("hz") {
        (number, PhysicalUnit::Hertz)
    } else {
        return Err(format!("physical value \"{text}\" has no recognized unit").into());
    };
    let value: f64 = number
        .parse()
        .map_err(|e| format!("Invalid physical value \"{text}\": {e}"))?;
    Ok((value, unit))
}

fn parse_movement_block(pair: Pair<Rule>) -> Result<MovementLimits, Box<dyn Error>> {
    let mut movement = MovementLimits::default();
    for param in pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::movement_param)
    {
        let mut name = String::new();
        let mut value = None;
        for inner in param.into_inner() {
            match inner.as_rule() {
                Rule::movement_param_name => name = inner.as_str().to_string(),
                Rule::speed_value => {
                    let text = inner.as_str().trim();
                    let number = text
                        .strip_suffix("deg/s")
                        .ok_or_else(|| format!("speed \"{text}\" must end in deg/s"))?;
                    value = Some(
                        number
                            .parse::<f64>()
                            .map_err(|e| format!("Invalid speed \"{text}\": {e}"))?,
                    );
                }
                _ => {}
            }
        }
        match name.as_str() {
            "max_pan_speed" => movement.max_pan_speed = value,
            "max_tilt_speed" => movement.max_tilt_speed = value,
            _ => {}
        }
    }
    Ok(movement)
}

fn parse_vec3(pair: Pair<Rule>) -> Result<Vec3, Box<dyn Error>> {
    let mut numbers = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::signed_number)
        .map(|p| p.as_str().trim().parse::<f64>());
    let x = numbers.next().ok_or("vec3 requires x")??;
    let y = numbers.next().ok_or("vec3 requires y")??;
    let z = numbers.next().ok_or("vec3 requires z")??;
    Ok([x, y, z])
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
    let mut focus_points = HashMap::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::venue_content => {
                parse_venue_content(pair, &mut fixtures, &mut focus_points)?;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Venue name is required".into());
    }

    let mut venue = Venue::new(name, fixtures);
    venue.set_focus_points(focus_points);
    Ok(venue)
}

fn parse_venue_content(
    pair: Pair<Rule>,
    fixtures: &mut HashMap<String, Fixture>,
    focus_points: &mut HashMap<String, Vec3>,
) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::fixture => {
                let fixture = parse_fixture_definition(content_pair)?;
                fixtures.insert(fixture.name().to_string(), fixture);
            }
            Rule::focus => {
                let mut name = String::new();
                let mut point = None;
                for inner in content_pair.into_inner() {
                    match inner.as_rule() {
                        Rule::string => name = extract_string(inner),
                        Rule::vec3 => point = Some(parse_vec3(inner)?),
                        _ => {}
                    }
                }
                let point = point.ok_or_else(|| {
                    format!("focus \"{name}\" requires coordinates, e.g. (0, 2.8, 1.4)")
                })?;
                focus_points.insert(name, point);
            }
            // `group` still parses so this can say what to do about it. Venue
            // groups were superseded by fixture tags one release after they
            // landed; a bare pest failure here would only say "expected
            // fixture", which is no help to someone holding a rig file.
            Rule::group => {
                let name = content_pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::string)
                    .map(extract_string)
                    .unwrap_or_default();
                return Err(format!(
                    "venue group `{name}` is no longer supported. Tag the fixtures \
                     instead — add a tag to each member, e.g. `tags [\"{name}\"]`, \
                     then declare a logical group under `dmx.lighting.groups` in \
                     the player config:\n\n{}\n\
                     Tags survive a venue change; venue groups did not.",
                    migration_yaml(&name)
                )
                .into());
            }
            _ => {}
        }
    }
    Ok(())
}

/// The config the venue-group migration message tells the reader to write.
///
/// Extracted so a test can feed the *actual* advice to the loader rather than a
/// copy of it. `groups` is a map keyed by name, and an earlier version of this
/// message printed a sequence — following it produced a config that would not
/// load, which is worse than the error it replaced. A test that parses its own
/// hand-written string cannot catch that.
pub(crate) fn migration_yaml(name: &str) -> String {
    format!(
        "    groups:\n      {name}:\n        name: {name}\n        \
         constraints:\n          - AllOf: [\"{name}\"]\n"
    )
}

pub(crate) fn parse_fixture_definition(pair: Pair<Rule>) -> Result<Fixture, Box<dyn Error>> {
    let mut name: Option<String> = None;
    let mut fixture_type = String::new();
    let mut universe = 0u16;
    let mut start_channel = 0u16;
    let mut tags = Vec::new();
    let mut position = None;
    let mut rotation = None;

    for pair in pair.into_inner() {
        match pair.as_rule() {
            // Positional: the first quoted value is the fixture's name, a
            // second is its type. Only the type may be given either way.
            Rule::string => match name {
                None => name = Some(extract_string(pair)),
                Some(_) => fixture_type = extract_string(pair),
            },
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
            Rule::fixture_position => {
                let vec3 = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::vec3)
                    .ok_or("position requires coordinates")?;
                position = Some(parse_vec3(vec3)?);
            }
            Rule::fixture_rotation => {
                let vec3 = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::vec3)
                    .ok_or("rotation requires coordinates")?;
                rotation = Some(parse_vec3(vec3)?);
            }
            _ => {}
        }
    }

    let mut fixture = Fixture::new(
        name.unwrap_or_default(),
        fixture_type,
        universe,
        start_channel,
        tags,
    );
    fixture.set_position(position);
    fixture.set_rotation(rotation);
    Ok(fixture)
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
    fn venue_group_is_rejected_with_migration_advice() {
        // Venue groups were superseded by fixture tags (#104, one release after
        // they landed) and are gone. The error has to be actionable: this is a
        // rig file someone may be reading at soundcheck.
        let content = r#"venue "Stage" {
    fixture "L1" Par @ 1:1
    fixture "L2" Par @ 1:5
    group "front" = L1, L2
}"#;
        let msg = match parse_venues(content) {
            Ok(_) => panic!("venue groups should be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(msg.contains("front"), "should name the group: {msg}");
        assert!(msg.contains("tags"), "should point at tags: {msg}");
        assert!(
            msg.contains("dmx.lighting.groups"),
            "should point at logical groups: {msg}"
        );
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

    // ── v2 fixture types ─────────────────────────────────────────

    #[test]
    fn fixture_type_v2_channel_one_liners() {
        let content = r#"fixture_type "House-Blinder" {
    channels: 4
    channel "dimmer" @ 1 fine 2
    channel "red"    @ 3
    channel "strobe" @ 4 {
        functions: { "off": 0..7, "strobe": 64..255 -> 0.3hz..25.0hz }
    }
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("House-Blinder").unwrap();
        assert_eq!(ft.channels().get("dimmer"), Some(&1));
        assert_eq!(ft.channels().get("red"), Some(&3));
        assert_eq!(ft.channels().get("strobe"), Some(&4));
        assert_eq!(ft.channel_defs().get("dimmer").unwrap().fine, Some(2));
        assert_eq!(ft.footprint(), 4);

        // The strobe function fills the legacy fields for v1 consumers.
        assert_eq!(ft.strobe_dmx_offset(), Some(64));
        assert_eq!(ft.min_strobe_frequency(), Some(0.3));
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));

        let strobe = ft.channel_defs().get("strobe").unwrap();
        assert_eq!(strobe.functions.len(), 2);
        assert_eq!(strobe.functions[0].name, "off");
        assert_eq!(strobe.functions[0].dmx_to, 7);
        assert!(strobe.functions[0].physical.is_none());
    }

    #[test]
    fn fixture_type_v2_range() {
        let content = r#"fixture_type "Mover" {
    channel "pan"  @ 1 fine 2 range: -270deg..270deg
    channel "tilt" @ 3 fine 4 range: -135deg..135deg
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Mover").unwrap();
        let pan = ft.channel_defs().get("pan").unwrap();
        let range = pan.range.unwrap();
        assert_eq!(range.from, -270.0);
        assert_eq!(range.to, 270.0);
        assert_eq!(range.unit, PhysicalUnit::Degrees);
        assert_eq!(ft.footprint(), 4);
    }

    #[test]
    fn fixture_type_v2_referential_with_movement() {
        let content = r#"fixture_type "Robe-Esprite"
  from gdtf("library/Robe@Esprite@V1.1.gdtf", mode "Mode 1")
{
    movement { max_pan_speed: 240deg/s  max_tilt_speed: 200deg/s }
}"#;
        let result = parse_fixture_types(content).unwrap();
        let ft = result.get("Robe-Esprite").unwrap();
        let source = ft.source().unwrap();
        assert_eq!(source.path, "library/Robe@Esprite@V1.1.gdtf");
        assert_eq!(source.mode, "Mode 1");
        assert_eq!(ft.movement().max_pan_speed, Some(240.0));
        assert_eq!(ft.movement().max_tilt_speed, Some(200.0));
        assert!(ft.channels().is_empty());
    }

    #[test]
    fn fixture_type_v2_mixed_units_rejected() {
        let content = r#"fixture_type "Bad" {
    channel "pan" @ 1 range: -270deg..25hz
}"#;
        let err = parse_fixture_types(content).unwrap_err().to_string();
        assert!(err.contains("mixes units"), "{err}");
    }

    #[test]
    fn fixture_type_v2_inverted_dmx_range_rejected() {
        let content = r#"fixture_type "Bad" {
    channel "strobe" @ 1 {
        functions: { "strobe": 200..100 }
    }
}"#;
        let err = parse_fixture_types(content).unwrap_err().to_string();
        assert!(err.contains("inverted"), "{err}");
    }

    #[test]
    fn fixture_type_v1_display_reparses_as_v2() {
        // The migration path: parse a v1 file, Display it (v2 form), and the
        // result must parse back to an equivalent fixture type.
        let v1 = r#"fixture_type "Astera-PixelBrick" {
  channels: 4
  channel_map: {
    "red": 1,
    "green": 2,
    "blue": 3,
    "strobe": 4
  }
  max_strobe_frequency: 25.0
  min_strobe_frequency: 0.4
  strobe_dmx_offset: 7
}"#;
        let parsed_v1 = parse_fixture_types(v1).unwrap();
        let original = parsed_v1.get("Astera-PixelBrick").unwrap();

        let rewritten = original.to_string();
        let parsed_v2 = parse_fixture_types(&rewritten)
            .unwrap_or_else(|e| panic!("v2 rewrite failed to parse: {e}\n{rewritten}"));
        let roundtrip = parsed_v2.get("Astera-PixelBrick").unwrap();

        assert_eq!(roundtrip.channels(), original.channels());
        assert_eq!(
            roundtrip.max_strobe_frequency(),
            original.max_strobe_frequency()
        );
        assert_eq!(
            roundtrip.min_strobe_frequency(),
            original.min_strobe_frequency()
        );
        assert_eq!(roundtrip.strobe_dmx_offset(), original.strobe_dmx_offset());
    }

    // ── v2 venues ────────────────────────────────────────────────

    #[test]
    fn venue_v2_positions_and_focus() {
        let content = r#"venue "kellys-basement" {
    fixture "Spot1" "Robe-Esprite" @ 1:1
        tags ["spot", "rear"]
        position (-2.0, 3.5, 4.2)  rotation (0, 0, 180)

    focus "drummer"      (0.0, 2.8, 1.4)
    focus "center-stage" (0.0, 1.5, 1.7)
}"#;
        let result = parse_venues(content).unwrap();
        let v = result.get("kellys-basement").unwrap();

        let spot = v.fixtures().get("Spot1").unwrap();
        assert_eq!(spot.position(), Some([-2.0, 3.5, 4.2]));
        assert_eq!(spot.rotation(), Some([0.0, 0.0, 180.0]));
        assert_eq!(spot.tags(), &["spot", "rear"]);

        assert_eq!(v.focus_points().len(), 2);
        assert_eq!(v.focus_points().get("drummer"), Some(&[0.0, 2.8, 1.4]));
    }

    #[test]
    fn venue_v2_display_reparses() {
        let content = r#"venue "Club" {
    fixture "Spot1" Par @ 1:1 tags ["front"] position (-2.0, 3.5, 4.2)
    focus "drummer" (0.0, 2.8, 1.4)
}"#;
        let parsed = parse_venues(content).unwrap();
        let original = parsed.get("Club").unwrap();

        let rewritten = original.to_string();
        let reparsed = parse_venues(&rewritten)
            .unwrap_or_else(|e| panic!("venue rewrite failed to parse: {e}\n{rewritten}"));
        let roundtrip = reparsed.get("Club").unwrap();

        let spot = roundtrip.fixtures().get("Spot1").unwrap();
        assert_eq!(spot.position(), Some([-2.0, 3.5, 4.2]));
        assert_eq!(roundtrip.focus_points(), original.focus_points());
    }

    #[test]
    fn venue_position_without_rotation() {
        let content = r#"venue "V" {
    fixture "F" Par @ 1:1 position (1, 2, 3)
}"#;
        let result = parse_venues(content).unwrap();
        let f = result.get("V").unwrap().fixtures().get("F").unwrap();
        assert_eq!(f.position(), Some([1.0, 2.0, 3.0]));
        assert_eq!(f.rotation(), None);
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
