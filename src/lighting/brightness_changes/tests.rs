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

use super::*;
use crate::testutil::lighting::shows;

fn fixture(name: &str, dimmer: bool) -> FixtureTraits {
    FixtureTraits {
        name: name.to_string(),
        rgb: true,
        dimmer,
        strobe: false,
    }
}

/// `dim` is a colour fixture with a dimmer channel, `rgb` one without.
fn ctx() -> BrightnessContext {
    BrightnessContext {
        groups: HashMap::from([
            ("dim".to_string(), vec![fixture("d1", true)]),
            ("rgb".to_string(), vec![fixture("r1", false)]),
        ]),
    }
}

fn found(source: &str, kind: &str) -> Vec<Warning> {
    brightness_changes(&shows(source), &ctx())
        .into_iter()
        .filter(|w| w.kind == kind)
        .collect()
}

const UPPER: &str = "brightness_change_upper_colour";
const REPLACE: &str = "brightness_change_dimmer_replace";
const RUNS_ON: &str = "brightness_change_colour_runs_on";
const NO_COLOUR: &str = "brightness_change_level_without_colour";

// ── colour above a dimmer ──────────────────────────────────────

#[test]
fn colour_above_a_dimmer_is_reported_on_every_fixture() {
    for group in ["dim", "rgb"] {
        let source = format!(
            r#"
show "s" {{
    @00:01.000
    {group}: dimmer start_level: 1.0, end_level: 0.5, duration: 4s
    @00:02.000
    {group}: static color: "blue", duration: 2s, layer: midground
}}
"#
        );
        let warnings = found(&source, UPPER);
        assert_eq!(warnings.len(), 1, "{group}: {warnings:?}");
        assert_eq!(warnings[0].severity, Severity::Note);
        let message = &warnings[0].message;
        assert!(message.contains("show \"s\" at 2.000s"), "{message}");
        assert!(message.contains(&format!("`{group}`")), "{message}");
        assert!(message.contains("midground") && message.contains("background"));
    }
}

#[test]
fn a_multiply_colour_above_a_dimmer_is_not_reported() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static dimmer: 50%, duration: 4s
    @00:00.000
    dim: static color: "blue", duration: 4s, layer: foreground, blend_mode: multiply
}
"#;
    assert!(found(source, UPPER).is_empty());
}

#[test]
fn colour_and_dimmer_that_do_not_overlap_are_not_reported() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static dimmer: 50%, duration: 2s
    @00:03.000
    dim: static color: "blue", duration: 2s, layer: midground
}
"#;
    assert!(found(source, UPPER).is_empty());
}

#[test]
fn colour_and_dimmer_on_the_same_layer_are_not_reported() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static color: "blue", dimmer: 30%, duration: 4s
    @00:00.000
    dim: static dimmer: 50%, duration: 4s
}
"#;
    assert!(found(source, UPPER).is_empty());
}

#[test]
fn colour_beneath_a_dimmer_and_other_groups_are_not_reported() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static color: "blue", duration: 4s
    @00:00.000
    dim: static dimmer: 50%, duration: 4s, layer: foreground
    @00:00.000
    rgb: static color: "red", duration: 4s, layer: foreground
}
"#;
    assert!(found(source, UPPER).is_empty());
}

#[test]
fn a_dimmer_at_full_does_not_dim_the_colour_above_it() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static dimmer: 100%, duration: 4s
    @00:00.000
    dim: static color: "blue", duration: 4s, layer: foreground
}
"#;
    assert!(found(source, UPPER).is_empty());
}

// ── dimmer replace above a dimmer ──────────────────────────────

#[test]
fn a_dimmer_replace_over_a_dimmer_is_reported_on_rgb_only_fixtures() {
    let source = r#"
show "s" {
    @00:00.000
    rgb: dimmer start_level: 0.5, end_level: 0.5, duration: 4s
    @00:01.000
    rgb: dimmer start_level: 1.0, end_level: 0.2, duration: 2s, layer: foreground, blend_mode: replace
}
"#;
    let warnings = found(source, REPLACE);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("at 1.000s"));
    assert!(warnings[0].message.contains("r1"));
}

#[test]
fn a_dimmer_replace_is_not_reported_on_dimmer_channel_fixtures_or_as_multiply() {
    let on_dimmer_channel = r#"
show "s" {
    @00:00.000
    dim: dimmer start_level: 0.5, end_level: 0.5, duration: 4s
    @00:01.000
    dim: dimmer start_level: 1.0, end_level: 0.2, duration: 2s, layer: foreground, blend_mode: replace
}
"#;
    assert!(found(on_dimmer_channel, REPLACE).is_empty());
    let multiply = r#"
show "s" {
    @00:00.000
    rgb: dimmer start_level: 0.5, end_level: 0.5, duration: 4s
    @00:01.000
    rgb: dimmer start_level: 1.0, end_level: 0.2, duration: 2s, layer: foreground, blend_mode: multiply
}
"#;
    assert!(found(multiply, REPLACE).is_empty());
    let later = r#"
show "s" {
    @00:00.000
    rgb: dimmer start_level: 0.5, end_level: 0.5, duration: 1s
    @00:02.000
    rgb: dimmer start_level: 1.0, end_level: 0.2, duration: 2s, layer: foreground, blend_mode: replace
}
"#;
    assert!(found(later, REPLACE).is_empty());
}

// ── colour running on after its dimmer ends ────────────────────

#[test]
fn colour_that_outlasts_its_dimmer_is_reported_on_dimmer_channel_fixtures() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static color: "red", duration: 10s
    @00:00.000
    dim: dimmer start_level: 1.0, end_level: 0.0, duration: 4s, layer: midground, blend_mode: multiply
}
"#;
    let warnings = found(source, RUNS_ON);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("at 4.000s"));
    assert!(warnings[0].message.contains("d1"));
}

#[test]
fn colour_covered_to_its_end_or_on_rgb_only_fixtures_is_not_reported() {
    let covered = r#"
show "s" {
    @00:00.000
    dim: static color: "red", duration: 10s
    @00:00.000
    dim: dimmer start_level: 1.0, end_level: 0.0, duration: 4s, layer: midground, blend_mode: multiply
    @00:04.000
    dim: static dimmer: 30%, duration: 6s, layer: midground, blend_mode: multiply
}
"#;
    assert!(found(covered, RUNS_ON).is_empty());
    let rgb_only = r#"
show "s" {
    @00:00.000
    rgb: static color: "red", duration: 10s
    @00:00.000
    rgb: dimmer start_level: 1.0, end_level: 0.0, duration: 4s, layer: midground, blend_mode: multiply
}
"#;
    assert!(found(rgb_only, RUNS_ON).is_empty());
}

#[test]
fn colour_and_dimmer_on_the_same_layer_for_the_same_span_is_not_reported() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static color: "red", dimmer: 50%, duration: 4s
}
"#;
    assert!(found(source, RUNS_ON).is_empty());
    assert!(found(source, NO_COLOUR).is_empty());
}

#[test]
fn colour_that_was_never_dimmed_is_not_reported() {
    let source = r#"
show "s" {
    @00:00.000
    dim: static color: "red", duration: 4s
}
"#;
    assert!(found(source, RUNS_ON).is_empty());
}

// ── a level with no colour beneath it ──────────────────────────

#[test]
fn a_level_with_no_colour_is_reported() {
    let source = r#"
show "s" {
    @00:02.000
    rgb: pulse frequency: 2, duration: 4s, layer: foreground, blend_mode: multiply
}
"#;
    let warnings = found(source, NO_COLOUR);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("at 2.000s"));
    assert!(warnings[0].message.contains("white"));
}

#[test]
fn a_level_over_a_colour_is_not_reported_but_an_uncovered_stretch_is() {
    let covered = r#"
show "s" {
    @00:00.000
    rgb: static color: "red", duration: 4s
    @00:01.000
    rgb: dimmer start_level: 1.0, end_level: 0.5, duration: 2s, layer: midground
}
"#;
    assert!(found(covered, NO_COLOUR).is_empty());
    let partly = r#"
show "s" {
    @00:00.000
    rgb: static color: "red", duration: 2s
    @00:01.000
    rgb: dimmer start_level: 1.0, end_level: 0.5, duration: 4s, layer: midground
}
"#;
    let warnings = found(partly, NO_COLOUR);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].message.contains("at 2.000s"));
}

#[test]
fn a_colour_above_the_level_is_not_beneath_it() {
    let source = r#"
show "s" {
    @00:00.000
    rgb: static dimmer: 50%, duration: 4s
    @00:00.000
    rgb: static color: "red", duration: 4s, layer: foreground
}
"#;
    assert_eq!(found(source, NO_COLOUR).len(), 1);
}

#[test]
fn a_strobe_is_a_level_only_where_there_is_no_strobe_channel() {
    let source = r#"
show "s" {
    @00:00.000
    rgb: strobe frequency: 4, duration: 2s
}
"#;
    assert_eq!(found(source, NO_COLOUR).len(), 1);
    let mut with_channel = ctx();
    with_channel.groups.get_mut("rgb").unwrap()[0].strobe = true;
    assert!(brightness_changes(&shows(source), &with_channel).is_empty());
}

#[test]
fn groups_the_venue_does_not_resolve_are_not_checked() {
    let source = r#"
show "s" {
    @00:00.000
    nobody: dimmer start_level: 1.0, end_level: 0.5, duration: 4s
}
"#;
    assert!(brightness_changes(&shows(source), &ctx()).is_empty());
    assert!(brightness_changes(&shows(source), &BrightnessContext::default()).is_empty());
}

#[test]
fn a_foreground_replace_colour_is_reported_only_on_dimmer_channel_fixtures() {
    // RGB-only fixtures already let a foreground `replace` escape every
    // lower dimmer; only dimmer-channel fixtures change.
    for (group, expected) in [("dim", 1), ("rgb", 0)] {
        let source = format!(
            r#"
show "s" {{
    @00:00.000
    {group}: static color: "red", dimmer: 50%, duration: 4s
    @00:01.000
    {group}: static color: "blue", duration: 2s, layer: foreground
}}
"#
        );
        let warnings = found(&source, UPPER);
        assert_eq!(warnings.len(), expected, "{group}: {warnings:?}");
    }
}

#[test]
fn a_chase_or_pulse_that_replaces_over_a_dimmer_is_reported_on_rgb_only_fixtures() {
    for upper in [
        "chase pattern: linear, speed: 2.0, direction: left_to_right",
        "pulse frequency: 2",
    ] {
        let source = format!(
            r#"
show "s" {{
    @00:00.000
    rgb: static color: "red", dimmer: 45%, duration: 4s
    @00:00.000
    rgb: {upper}, duration: 4s, layer: midground
}}
"#
        );
        let warnings = found(&source, REPLACE);
        assert_eq!(warnings.len(), 1, "{upper}: {warnings:?}");
        assert!(warnings[0].message.contains("blend_mode: multiply"));

        let multiply = source.replace("layer: midground", "layer: midground, blend_mode: multiply");
        assert!(found(&multiply, REPLACE).is_empty(), "{upper} as multiply");
    }
}
