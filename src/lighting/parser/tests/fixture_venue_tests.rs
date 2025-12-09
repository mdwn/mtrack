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
use crate::lighting::parser::fixture_venue::parse_fixture_definition;
use crate::lighting::parser::grammar::{LightingParser, Rule};
use crate::lighting::parser::*;
use pest::Parser;

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
    // Note: special_cases field was removed from FixtureType
}

#[test]
fn test_t_parse_venue() {
    let content = r#"venue "Club Venue" { }"#;

    let result = parse_venues(content).unwrap();
    assert_eq!(result.len(), 1);

    let venue = result.get("Club Venue").unwrap();
    assert_eq!(venue.name(), "Club Venue");
    assert_eq!(venue.fixtures().len(), 0);
    assert_eq!(venue.groups().len(), 0);
}

#[test]
fn test_fixture_universe_address_parsing() {
    // Test that fixture parsing correctly extracts universe and address
    let fixture_content = r#"fixture "Block1" Astera-PixelBrick @ 1:1"#;

    // First test if the grammar can parse the fixture rule
    match LightingParser::parse(Rule::fixture, fixture_content) {
        Ok(mut pairs) => {
            if let Some(pair) = pairs.next() {
                let fixture =
                    parse_fixture_definition(pair).expect("Failed to parse fixture definition");
                assert_eq!(fixture.universe(), 1, "Block1 should be on universe 1");
                assert_eq!(fixture.start_channel(), 1, "Block1 should be at address 1");
            } else {
                panic!("No fixture pair found");
            }
        }
        Err(e) => {
            panic!("Failed to parse fixture: {:?}", e);
        }
    }
}

#[test]
fn test_venue_with_fixtures() {
    let content = r#"venue "test" {
    fixture "Block1" Astera-PixelBrick @ 1:1
    fixture "Block2" Astera-PixelBrick @ 1:5
}"#;

    let venues = parse_venues(content).expect("Failed to parse venue with fixtures");
    assert_eq!(venues.len(), 1);

    let venue = venues.get("test").expect("test venue not found");
    assert_eq!(venue.fixtures().len(), 2);

    let block1 = venue.fixtures().get("Block1").expect("Block1 not found");
    assert_eq!(block1.universe(), 1, "Block1 should be on universe 1");
    assert_eq!(block1.start_channel(), 1, "Block1 should be at address 1");

    let block2 = venue.fixtures().get("Block2").expect("Block2 not found");
    assert_eq!(block2.universe(), 1, "Block2 should be on universe 1");
    assert_eq!(block2.start_channel(), 5, "Block2 should be at address 5");
}

#[test]
fn test_venue_with_comments() {
    // Test venue with a comment at the beginning
    let content = r#"# The built-in venue represents the lights that come with our IEM rig.
venue "built-in" {
    fixture "Block1" Astera-PixelBrick @ 1:1 tags ["wash", "side"]  
    fixture "Block2" Astera-PixelBrick @ 1:5 tags ["wash", "side"]  
    fixture "Block3" Astera-PixelBrick @ 1:9 tags ["wash", "front"]  
    fixture "Block4" Astera-PixelBrick @ 1:13 tags ["wash", "front"]  
    fixture "Block5" Astera-PixelBrick @ 1:17 tags ["wash", "front"]  
    fixture "Block6" Astera-PixelBrick @ 1:21 tags ["wash", "front"]  
    fixture "Block7" Astera-PixelBrick @ 1:25 tags ["wash", "side"]  
    fixture "Block8" Astera-PixelBrick @ 1:29 tags ["wash", "side"]  
}"#;

    let venues = parse_venues(content).expect("Failed to parse venues with comments");
    assert_eq!(venues.len(), 1);

    let venue = venues.get("built-in").expect("built-in venue not found");
    assert_eq!(venue.name(), "built-in");
    assert_eq!(venue.fixtures().len(), 8);

    // Verify that fixtures have correct universe and address values
    let block3 = venue.fixtures().get("Block3").expect("Block3 not found");
    assert_eq!(block3.universe(), 1, "Block3 should be on universe 1");
    assert_eq!(block3.start_channel(), 9, "Block3 should be at address 9");

    let block4 = venue.fixtures().get("Block4").expect("Block4 not found");
    assert_eq!(block4.universe(), 1, "Block4 should be on universe 1");
    assert_eq!(block4.start_channel(), 13, "Block4 should be at address 13");

    let block5 = venue.fixtures().get("Block5").expect("Block5 not found");
    assert_eq!(block5.universe(), 1, "Block5 should be on universe 1");
    assert_eq!(block5.start_channel(), 17, "Block5 should be at address 17");

    let block6 = venue.fixtures().get("Block6").expect("Block6 not found");
    assert_eq!(block6.universe(), 1, "Block6 should be on universe 1");
    assert_eq!(block6.start_channel(), 21, "Block6 should be at address 21");
}
