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
fn test_parse_fixture_type_with_strobe_range() {
    let content = r#"fixture_type "Astera-PixelBrick" {
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

    let result = parse_fixture_types(content).unwrap();
    assert_eq!(result.len(), 1);

    let fixture_type = result.get("Astera-PixelBrick").unwrap();
    assert_eq!(fixture_type.max_strobe_frequency(), Some(25.0));
    assert_eq!(fixture_type.min_strobe_frequency(), Some(0.4));
    assert_eq!(fixture_type.strobe_dmx_offset(), Some(7));
}

#[test]
fn test_t_parse_venue() {
    let content = r#"venue "Club Venue" { }"#;

    let result = parse_venues(content).unwrap();
    assert_eq!(result.len(), 1);

    let venue = result.get("Club Venue").unwrap();
    assert_eq!(venue.name(), "Club Venue");
    assert_eq!(venue.fixtures().len(), 0);
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

// ── Venue groups are gone (superseded by tags) ─────────────────────────
//
// Venue groups were removed in favour of fixture tags plus logical groups.
// The grammar still matches the old syntax so the parser can say what to do
// about it — a bare pest failure would only report "expected fixture".

#[test]
fn multiple_venue_groups_are_rejected() {
    // This shape used to fail for an unrelated reason: `identifier` was not
    // atomic, so a member list ran past the end of its line and swallowed the
    // next keyword (#387). It now fails deliberately, and says why.
    let content = r#"venue "v" {
    fixture "P1" RGBW_Par @ 1:1
    fixture "P2" RGBW_Par @ 1:7
    group "all" = P1, P2
    group "left" = P1
}
"#;
    let msg = match parse_venues(content) {
        Ok(_) => panic!("venue groups should be rejected"),
        Err(e) => e.to_string(),
    };
    assert!(msg.contains("no longer supported"), "{msg}");
    assert!(msg.contains("tags"), "should point at the migration: {msg}");
}

#[test]
fn a_venue_of_only_fixtures_still_parses() {
    // The shape every real venue uses. This also covers the `identifier`
    // atomicity fix from #387: without it a fixture's type name could run past
    // the end of its line and swallow the next `fixture` keyword.
    let content = r#"venue "v" {
    fixture "P1" RGBW_Par @ 1:1 tags ["wash", "left"]
    fixture "P2" RGBW_Par @ 1:7 tags ["wash", "right"]
    fixture "P3" RGBW_Par @ 1:13
}
"#;
    let venues = parse_venues(content).expect("a fixtures-only venue parses");
    let venue = venues.get("v").expect("venue `v` not found");

    assert_eq!(venue.fixtures().len(), 3);
    assert_eq!(
        venue.fixtures().get("P1").expect("P1").tags(),
        ["wash", "left"]
    );
    assert_eq!(
        venue.fixtures().get("P3").expect("P3").fixture_type(),
        "RGBW_Par"
    );
}

#[test]
fn a_fixture_type_whose_name_is_not_a_bare_word_can_be_quoted() {
    // `identifier` is atomic, so `Moving Head` unquoted is two tokens and does
    // not parse — it only ever worked because a non-atomic rule swallowed the
    // space along with the word. Quoting says the same thing explicitly.
    let quoted = "venue \"v\" {\n  fixture \"M1\" \"Moving Head\" @ 1:1\n}\n";
    let venues = parse_venues(quoted).expect("a quoted type parses");
    let venue = venues.get("v").expect("venue");
    let fixture = venue.fixtures().get("M1").expect("fixture");
    assert_eq!(fixture.fixture_type(), "Moving Head");
    assert_eq!(fixture.name(), "M1");

    // The bare form still parses for ordinary names.
    let bare = "venue \"v\" {\n  fixture \"M1\" MovingHead @ 1:1\n}\n";
    let venues = parse_venues(bare).expect("a bare type parses");
    assert_eq!(venues["v"].fixtures()["M1"].fixture_type(), "MovingHead");
}

// ── .venue syntax: provenance, positions, focus points ───────────

#[test]
fn a_venue_carries_positions_rotations_and_focus_points() {
    let content = r#"venue "kellys-basement" {
  imported from mvr("lighting/library/kellys.mvr") origin (0, -3.5, 0)
  fixture "Spot1" "Robe Esprite" @ 1:1 tags ["spot", "rear"] position (-2.0, 3.5, 4.2) rotation (0, 0, 180)
  fixture "Wash1" RGBW_Par @ 1:40 position (1, 1, 1)
  fixture "Old1" RGBW_Par @ 1:60
  focus "drummer" (0.0, 2.8, 1.4)
  focus "center-stage" (0, 1.5, 1.7)
}"#;
    let venues = parse_venues(content).expect("parses");
    let venue = &venues["kellys-basement"];

    let source = venue.source().expect("provenance");
    assert_eq!(source.mvr, "lighting/library/kellys.mvr");
    assert_eq!(source.origin, [0.0, -3.5, 0.0]);

    let spot = &venue.fixtures()["Spot1"];
    assert_eq!(spot.fixture_type(), "Robe Esprite");
    assert_eq!(spot.tags(), ["spot", "rear"]);
    assert_eq!(spot.position(), Some([-2.0, 3.5, 4.2]));
    assert_eq!(spot.rotation(), Some([0.0, 0.0, 180.0]));

    let wash = &venue.fixtures()["Wash1"];
    assert!(wash.tags().is_empty());
    assert_eq!(wash.position(), Some([1.0, 1.0, 1.0]));
    assert_eq!(wash.rotation(), None);

    // A v1-shaped line is still a fixture without geometry.
    let old = &venue.fixtures()["Old1"];
    assert_eq!(old.position(), None);

    assert_eq!(venue.focus_points()["drummer"], [0.0, 2.8, 1.4]);
    assert_eq!(venue.focus_points()["center-stage"], [0.0, 1.5, 1.7]);
}

#[test]
fn fixture_attributes_may_come_in_any_order_but_only_once() {
    let any_order = "venue \"v\" {\n  fixture \"A\" T @ 1:1 position (1, 2, 3) tags [\"x\"]\n}\n";
    let venue = &parse_venues(any_order).expect("parses")["v"];
    assert_eq!(venue.fixtures()["A"].position(), Some([1.0, 2.0, 3.0]));
    assert_eq!(venue.fixtures()["A"].tags(), ["x"]);

    for attribute in ["tags [\"x\"]", "position (1, 2, 3)", "rotation (0, 0, 1)"] {
        let twice =
            format!("venue \"v\" {{\n  fixture \"A\" T @ 1:1 {attribute} {attribute}\n}}\n");
        let err = parse_venues(&twice).expect_err("a duplicate attribute is refused");
        assert!(err.to_string().contains("more than once"), "{err}");
    }
}

#[test]
fn duplicate_fixture_and_focus_names_are_refused() {
    let fixtures = "venue \"v\" {\n  fixture \"A\" T @ 1:1\n  fixture \"A\" T @ 1:5\n}\n";
    let err = parse_venues(fixtures).expect_err("duplicate fixture");
    assert!(
        err.to_string().contains("fixture \"A\" more than once"),
        "{err}"
    );

    let focus = "venue \"v\" {\n  focus \"d\" (0, 0, 0)\n  focus \"d\" (1, 1, 1)\n}\n";
    let err = parse_venues(focus).expect_err("duplicate focus");
    assert!(
        err.to_string().contains("focus point \"d\" more than once"),
        "{err}"
    );

    let source =
        "venue \"v\" {\n  imported from mvr(\"a.mvr\")\n  imported from mvr(\"b.mvr\")\n}\n";
    let err = parse_venues(source).expect_err("duplicate source");
    assert!(err.to_string().contains("more than once"), "{err}");
}

#[test]
fn an_import_without_an_origin_defaults_to_zero() {
    let content = "venue \"v\" {\n  imported from mvr(\"lighting/library/v.mvr\")\n}\n";
    let venue = &parse_venues(content).expect("parses")["v"];
    assert_eq!(venue.source().unwrap().origin, [0.0, 0.0, 0.0]);
}

#[test]
fn the_venue_display_form_round_trips_through_the_parser() {
    let content = r#"venue "v" {
  imported from mvr("lighting/library/v.mvr") origin (0.5, -3.25, 0)
  fixture "Spot1" "Robe Esprite" @ 1:1 tags ["spot"] position (-2, 3.5, 4.2) rotation (0, 0, 180)
  fixture "Wash1" RGBW_Par @ 1:40 position (1.0005, -0.0004, 1)
  focus "drummer" (0, 2.8, 1.4)
}"#;
    let venue = &parse_venues(content).expect("parses")["v"];
    let rendered = venue.to_string();
    let again = &parse_venues(&rendered).expect("the rendered form parses")["v"];

    assert_eq!(again.source(), venue.source());
    assert_eq!(again.focus_points(), venue.focus_points());
    assert_eq!(
        again.fixtures()["Spot1"].to_string(),
        venue.fixtures()["Spot1"].to_string()
    );
    // Coordinates are written to millimeter precision, without negative zero.
    assert_eq!(
        again.fixtures()["Wash1"].position(),
        Some([1.001, 0.0, 1.0])
    );
    assert!(rendered.contains("position (1.001, 0, 1)"), "{rendered}");
    assert!(
        rendered.contains("\"Robe Esprite\""),
        "a type with a space is quoted"
    );
    assert!(
        rendered.contains(" RGBW_Par @"),
        "a bare-word type stays bare"
    );
}
