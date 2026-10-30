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
//! Sequences with parameters bound where they are used (#540).

use crate::lighting::effects::EffectType;
use crate::lighting::parser::*;
use std::time::Duration;

const TEMPO: &str = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}
"#;

/// The (red, green, blue) a static effect on `group` sets in the cue at `at`.
fn rgb_at(show: &LightShow, at: Duration, group: &str) -> (f64, f64, f64) {
    let effect = show
        .cues
        .iter()
        .filter(|c| c.time == at)
        .flat_map(|c| c.effects.iter())
        .find(|e| e.groups == vec![group.to_string()])
        .unwrap_or_else(|| panic!("no effect on {group} at {at:?}"));
    match &effect.effect_type {
        EffectType::Static { parameters, .. } => {
            (parameters["red"], parameters["green"], parameters["blue"])
        }
        other => panic!("expected a static, got {other:?}"),
    }
}

fn parse(body: &str) -> Result<crate::lighting::parser::LightShows, String> {
    parse_light_shows(&format!("{TEMPO}{body}")).map_err(|e| e.to_string())
}

#[test]
fn arguments_override_defaults_and_bind_groups() {
    let shows = parse(
        r##"
sequence "strike-2"(hit: "#FFFFFF", bed: "#1E6FE0", target: all_wash) {
    @1/1
    $target: static, color: $bed, layer: background, blend_mode: replace, duration: 2measures
    @1/1
    $target: static, color: $hit, layer: foreground, blend_mode: replace, duration: 1beat
}

show "Song" {
    @1/1
    sequence "strike-2"
    @3/1
    sequence "strike-2"(hit: "#FF0000", target: front_wash)
}
"##,
    )
    .expect("parses");
    let show = &shows["Song"];

    let bar1 = Duration::ZERO;
    let bar3 = Duration::from_secs(4);
    // Defaults: white on blue, on all_wash.
    let first: Vec<_> = show
        .cues
        .iter()
        .filter(|c| c.time == bar1)
        .flat_map(|c| c.effects.iter())
        .collect();
    assert_eq!(first.len(), 2);
    assert!(first
        .iter()
        .all(|e| e.groups == vec!["all_wash".to_string()]));

    // Overridden: red hit on front_wash, the bed still its default blue.
    let second: Vec<_> = show
        .cues
        .iter()
        .filter(|c| c.time == bar3)
        .flat_map(|c| c.effects.iter())
        .collect();
    assert_eq!(second.len(), 2);
    assert!(second
        .iter()
        .all(|e| e.groups == vec!["front_wash".to_string()]));
    let colours: Vec<(f64, f64, f64)> = second
        .iter()
        .map(|e| match &e.effect_type {
            EffectType::Static { parameters, .. } => {
                (parameters["red"], parameters["green"], parameters["blue"])
            }
            _ => unreachable!(),
        })
        .collect();
    assert!(colours.contains(&(1.0, 0.0, 0.0)), "{colours:?}");
    assert!(
        colours
            .iter()
            .any(|(r, g, b)| (*r - 30.0 / 255.0).abs() < 1e-9
                && (*g - 111.0 / 255.0).abs() < 1e-9
                && (*b - 224.0 / 255.0).abs() < 1e-9),
        "{colours:?}"
    );
    // Every instance stops under the sequence's own name.
    assert!(show
        .cues
        .iter()
        .flat_map(|c| c.effects.iter())
        .all(|e| e.sequence_name.as_deref() == Some("strike-2")));
}

#[test]
fn parameters_pass_through_nested_sequences() {
    let shows = parse(
        r##"
sequence "inner"(colour) {
    @1/1
    front_wash: static, color: $colour, duration: 1beat
}

sequence "outer"(hit: "#00FF00", times: 2) {
    @1/1
    sequence "inner"(colour: $hit), loop: $times
}

show "Song" {
    @1/1
    sequence "outer"(hit: "#0000FF")
}
"##,
    )
    .expect("parses");
    let show = &shows["Song"];
    assert_eq!(show.cues.len(), 2, "loop: $times bound to 2");
    assert_eq!(rgb_at(show, Duration::ZERO, "front_wash"), (0.0, 0.0, 1.0));
    assert_eq!(
        rgb_at(show, Duration::from_millis(500), "front_wash"),
        (0.0, 0.0, 1.0)
    );
}

#[test]
fn levels_and_durations_substitute() {
    let shows = parse(
        r#"
sequence "glow"(level: 50%, hold: 2beats) {
    @1/1
    front_wash: static, color: "red", dimmer: $level, duration: $hold
}

show "Song" {
    @1/1
    sequence "glow"(level: 25%, hold: 1measure)
}
"#,
    )
    .expect("parses");
    let effect = &shows["Song"].cues[0].effects[0];
    match &effect.effect_type {
        EffectType::Static { parameters, .. } => assert_eq!(parameters["dimmer"], 0.25),
        _ => unreachable!(),
    }
    assert_eq!(effect.effect_type.duration(), Duration::from_secs(2));
}

#[test]
fn a_required_parameter_must_be_bound_where_used() {
    let body = r#"
sequence "hit"(colour) {
    @1/1
    front_wash: static, color: $colour, duration: 1beat
}

show "Song" {
    @1/1
    sequence "hit"
}
"#;
    let err = parse(body).expect_err("unbound parameter");
    // Line 15: the reference in the show, counting the tempo block.
    assert!(err.contains("line 15"), "names the call's line: {err}");
    assert!(err.contains("'colour'"), "names the parameter: {err}");

    // Defined and never used: nothing to check it against, so no error.
    let unused = body.replace("    sequence \"hit\"\n", "");
    parse(&unused).expect("an unused sequence with a required parameter parses");
}

#[test]
fn unknown_and_duplicate_arguments_are_errors() {
    let def = r#"
sequence "hit"(colour: "red") {
    @1/1
    front_wash: static, color: $colour, duration: 1beat
}
"#;
    let err = parse(&format!(
        "{def}show \"Song\" {{\n    @1/1\n    sequence \"hit\"(color: \"blue\")\n}}\n"
    ))
    .expect_err("unknown parameter");
    assert!(
        err.contains("no parameter 'color'") && err.contains("colour"),
        "{err}"
    );

    let err = parse(&format!(
        "{def}show \"Song\" {{\n    @1/1\n    sequence \"hit\"(colour: \"blue\", colour: \"red\")\n}}\n"
    ))
    .expect_err("duplicate argument");
    assert!(err.contains("twice"), "{err}");

    let err = parse(
        "sequence \"plain\" {\n    @0.000\n    front_wash: static, color: \"red\", duration: 1s\n}\n\
         show \"Song\" {\n    @0.000\n    sequence \"plain\"(colour: \"blue\")\n}\n",
    )
    .expect_err("arguments to a sequence without parameters");
    assert!(err.contains("takes no parameters"), "{err}");
}

#[test]
fn an_undeclared_parameter_is_an_error_at_its_line() {
    let err = parse(
        r#"
sequence "hit"(colour: "red") {
    @1/1
    front_wash: static, color: $color, duration: 1beat
}
"#,
    )
    .expect_err("undeclared $color");
    assert!(err.contains("line 10") && err.contains("$color"), "{err}");
}

#[test]
fn a_bad_value_is_reported_at_the_call_and_in_the_body() {
    let err = parse(
        r##"
sequence "hit"(colour: "red") {
    @1/1
    front_wash: static, color: $colour, duration: 1beat
}

show "Song" {
    @1/1
    sequence "hit"(colour: "#X")
}
"##,
    )
    .expect_err("bad colour after substitution");
    assert!(err.contains("line 15"), "the call's line: {err}");
    assert!(err.contains("line 10"), "the body's line: {err}");
    assert!(err.contains("#X"), "the value: {err}");

    // A default is checked even if the sequence is never used.
    let err = parse(
        r##"
sequence "hit"(colour: "#X") {
    @1/1
    front_wash: static, color: $colour, duration: 1beat
}
"##,
    )
    .expect_err("bad default");
    assert!(
        err.contains("default parameters") && err.contains("#X"),
        "{err}"
    );
}

#[test]
fn a_parameter_outside_a_sequence_is_an_error() {
    let err = parse(
        r#"
show "Song" {
    @1/1
    $target: static, color: "red", duration: 1beat
}
"#,
    )
    .expect_err("$target in a show");
    assert!(err.contains("line 10") && err.contains("$target"), "{err}");
}

#[test]
fn each_distinct_binding_is_its_own_instance() {
    // Two instances back to back, each with its own colour, and a third
    // reusing the first's binding.
    let shows = parse(
        r##"
sequence "one"(c) {
    @1/1
    front_wash: static, color: $c, duration: 1measure
}

show "Song" {
    @1/1
    sequence "one"(c: "#FF0000")
    @2/1
    sequence "one"(c: "#00FF00")
    @3/1
    sequence "one"(c: "#FF0000")
}
"##,
    )
    .expect("parses");
    let show = &shows["Song"];
    assert_eq!(rgb_at(show, Duration::ZERO, "front_wash"), (1.0, 0.0, 0.0));
    assert_eq!(
        rgb_at(show, Duration::from_secs(2), "front_wash"),
        (0.0, 1.0, 0.0)
    );
    assert_eq!(
        rgb_at(show, Duration::from_secs(4), "front_wash"),
        (1.0, 0.0, 0.0)
    );
}
