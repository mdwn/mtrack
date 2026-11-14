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
//! Palettes: a `palette { }` block of colour slots and the `palette.<slot>`
//! references to them.

use std::collections::HashMap;

use crate::lighting::effects::{Color, EffectType};
use crate::lighting::parser::document::document;
use crate::lighting::parser::writer::write;
use crate::lighting::parser::*;

const TEMPO: &str = r#"
tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}
"#;

const PALETTE: &str = r##"
palette {
    primary: "#C01020"
    secondary: "orange"
    accent: rgb(255, 240, 200)
}
"##;

fn parse(body: &str) -> Result<LightShows, String> {
    parse_light_shows(&format!("{TEMPO}{body}")).map_err(|e| e.to_string())
}

fn show(shows: &LightShows) -> &LightShow {
    shows.values().next().expect("a show")
}

fn effects(show: &LightShow) -> Vec<&Effect> {
    show.cues.iter().flat_map(|c| c.effects.iter()).collect()
}

fn cycle_colors(effect: &Effect) -> Vec<Color> {
    match &effect.effect_type {
        EffectType::ColorCycle { colors, .. } => colors.clone(),
        other => panic!("expected a cycle, got {other:?}"),
    }
}

fn rgb(effect: &Effect) -> (u8, u8, u8) {
    match &effect.effect_type {
        EffectType::Static { parameters, .. } => (
            (parameters["red"] * 255.0).round() as u8,
            (parameters["green"] * 255.0).round() as u8,
            (parameters["blue"] * 255.0).round() as u8,
        ),
        other => panic!("expected a static, got {other:?}"),
    }
}

const PRIMARY: Color = Color {
    r: 0xC0,
    g: 0x10,
    b: 0x20,
    w: None,
};
const SECONDARY: Color = Color {
    r: 255,
    g: 165,
    b: 0,
    w: None,
};
const BLUE: Color = Color {
    r: 0x20,
    g: 0x40,
    b: 0xFF,
    w: None,
};

fn overrides(pairs: &[(&str, Color)]) -> HashMap<String, Color> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

// ── the block ─────────────────────────────────────────────────────────

#[test]
fn a_palette_block_declares_slots_with_default_colors() {
    let shows = parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    a: static, color: palette.primary, duration: 1beat\n}}\n"
    ))
    .unwrap();
    let palette = &show(&shows).palette;
    let slots: Vec<&str> = palette.keys().map(String::as_str).collect();
    assert_eq!(slots, ["primary", "secondary", "accent"]);
    assert_eq!(palette["primary"], PRIMARY);
    assert_eq!(palette["secondary"], SECONDARY);
    assert_eq!(palette["accent"], Color::new(255, 240, 200));
}

#[test]
fn slots_may_be_separated_by_commas_or_lines() {
    for block in [
        "palette { a: \"red\", b: \"blue\" }",
        "palette { a: \"red\" b: \"blue\" }",
        "palette {\n  a: \"red\",\n  b: \"blue\",\n}",
        "palette {\n  a: \"red\" # one\n  // two\n  b: \"blue\"\n}",
    ] {
        let shows = parse(&format!(
            "{block}\nshow \"S\" {{\n    @1/1\n    a: static, color: palette.b, duration: 1beat\n}}\n"
        ))
        .unwrap_or_else(|e| panic!("{block}: {e}"));
        assert_eq!(show(&shows).palette.len(), 2, "{block}");
    }
}

#[test]
fn a_second_palette_block_is_an_error() {
    let err = parse(&format!(
        "{PALETTE}\npalette {{ other: \"red\" }}\nshow \"S\" {{\n    @1/1\n    a: static, color: \"red\", duration: 1beat\n}}\n"
    ))
    .unwrap_err();
    assert!(err.contains("second palette block"), "{err}");
    assert!(err.contains("line"), "{err}");
}

#[test]
fn a_hex_color_is_quoted_as_it_is_for_color() {
    // A bare `#` begins a comment, here as in `color: #C01020`.
    let shows = parse(
        "palette { a: \"#C01020\" }\nshow \"S\" {\n    @1/1\n    a: static, color: palette.a, duration: 1beat\n}\n",
    )
    .unwrap();
    assert_eq!(show(&shows).palette["a"], PRIMARY);
    assert!(parse(
        "palette { a: #C01020 }\nshow \"S\" {\n    @1/1\n    a: static, color: palette.a, duration: 1beat\n}\n"
    )
    .is_err());
}

#[test]
fn a_slot_declared_twice_is_an_error() {
    let err = parse("palette { a: \"red\", a: \"blue\" }\nshow \"S\" {\n    @1/1\n    a: static, color: \"red\", duration: 1beat\n}\n")
        .unwrap_err();
    assert!(err.contains("'a' twice"), "{err}");
}

#[test]
fn a_slot_with_a_bad_color_is_an_error() {
    let err = parse("palette { a: rgb(300, 0, 0) }\nshow \"S\" {\n    @1/1\n    a: static, color: palette.a, duration: 1beat\n}\n")
        .unwrap_err();
    assert!(err.contains("palette slot 'a'"), "{err}");
}

// ── references ────────────────────────────────────────────────────────

#[test]
fn a_static_color_can_be_a_slot() {
    let shows = parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    a: static, color: palette.primary, duration: 1beat\n}}\n"
    ))
    .unwrap();
    let effect = effects(show(&shows))[0];
    assert_eq!(rgb(effect), (0xC0, 0x10, 0x20));
    assert_eq!(
        effect.color_refs,
        vec![ColorRef::Slot("primary".to_string())]
    );
}

#[test]
fn a_cycle_color_list_can_mix_slots_and_literals() {
    let shows = parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    a: cycle, color: palette.primary, color: \"blue\", color: palette.secondary, speed: 1.0, duration: 4beats\n}}\n"
    ))
    .unwrap();
    let effect = effects(show(&shows))[0];
    assert_eq!(
        cycle_colors(effect),
        vec![PRIMARY, Color::new(0, 0, 255), SECONDARY]
    );
    assert_eq!(
        effect.color_refs,
        vec![
            ColorRef::Slot("primary".to_string()),
            ColorRef::Literal(Color::new(0, 0, 255)),
            ColorRef::Slot("secondary".to_string()),
        ]
    );
}

#[test]
fn a_reference_works_without_a_comma_after_the_effect_type() {
    let shows = parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    a: static color: palette.accent, duration: 1beat\n}}\n"
    ))
    .unwrap();
    assert_eq!(rgb(effects(show(&shows))[0]), (255, 240, 200));
}

#[test]
fn a_slot_can_be_passed_as_a_sequence_argument() {
    let shows = parse(&format!(
        r#"{PALETTE}
sequence "hit"(color: "red") {{
    @1/1
    a: static, color: $color, duration: 1beat
}}

show "S" {{
    @1/1
    sequence "hit"(color: palette.accent)
    @2/1
    sequence "hit"
}}
"#
    ))
    .unwrap();
    let all = effects(show(&shows));
    assert_eq!(all.len(), 2);
    assert_eq!(rgb(all[0]), (255, 240, 200));
    assert_eq!(
        all[0].color_refs,
        vec![ColorRef::Slot("accent".to_string())]
    );
    assert_eq!(rgb(all[1]), (255, 0, 0));
}

#[test]
fn a_sequence_body_can_name_a_slot_itself_and_as_a_default() {
    let shows = parse(&format!(
        r#"{PALETTE}
sequence "bed"(color: palette.secondary) {{
    @1/1
    a: static, color: $color, duration: 1beat
    b: static, color: palette.primary, duration: 1beat
}}

show "S" {{
    @1/1
    sequence "bed"
}}
"#
    ))
    .unwrap();
    let all = effects(show(&shows));
    assert_eq!(rgb(all[0]), (255, 165, 0));
    assert_eq!(rgb(all[1]), (0xC0, 0x10, 0x20));
}

#[test]
fn a_slot_inside_a_step_list_parses() {
    // Steps are in the grammar's parameter values, though no effect reads
    // them yet: a slot is as good a value as any other there.
    assert!(parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    a: static, steps: [{{color: palette.primary}}], duration: 1beat\n}}\n"
    ))
    .is_ok());
}

#[test]
fn an_undeclared_slot_is_an_error_naming_it_and_its_place() {
    let err = parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    a: static, color: palette.missing, duration: 1beat\n}}\n"
    ))
    .unwrap_err();
    assert!(
        err.contains("palette slot 'missing' is not declared"),
        "{err}"
    );
    assert!(err.contains("line 16"), "{err}");
    assert!(err.contains("column"), "{err}");
    assert!(
        err.contains("declared: primary, secondary, accent"),
        "{err}"
    );
}

#[test]
fn a_reference_without_a_palette_block_is_an_error() {
    let err =
        parse("show \"S\" {\n    @1/1\n    a: static, color: palette.x, duration: 1beat\n}\n")
            .unwrap_err();
    assert!(err.contains("palette slot 'x' is not declared"), "{err}");
    assert!(err.contains("no palette block"), "{err}");
}

#[test]
fn an_undeclared_slot_passed_to_a_sequence_is_an_error_where_it_is_used() {
    let err = parse(&format!(
        r#"{PALETTE}
sequence "hit"(color) {{
    @1/1
    a: static, color: $color, duration: 1beat
}}

show "S" {{
    @1/1
    sequence "hit"(color: palette.nope)
}}
"#
    ))
    .unwrap_err();
    assert!(err.contains("palette slot 'nope' is not declared"), "{err}");
}

// ── resolution ────────────────────────────────────────────────────────

const TWO_USES: &str = r#"
show "S" {
    @1/1
    a: static, color: palette.primary, duration: 1beat
    b: cycle, color: palette.primary, color: palette.secondary, speed: 1.0, duration: 1beat
}
"#;

fn parsed(body: &str) -> Vec<LightShow> {
    parse(&format!("{PALETTE}{body}"))
        .unwrap()
        .into_values()
        .collect()
}

#[test]
fn with_no_overrides_the_defaults_are_the_colors() {
    let shows = resolve_shows(parsed(TWO_USES), &HashMap::new());
    let all = effects(&shows[0]);
    assert_eq!(rgb(all[0]), (0xC0, 0x10, 0x20));
    assert_eq!(cycle_colors(all[1]), vec![PRIMARY, SECONDARY]);
}

#[test]
fn an_override_wins_per_slot() {
    let shows = resolve_shows(parsed(TWO_USES), &overrides(&[("primary", BLUE)]));
    let all = effects(&shows[0]);
    assert_eq!(rgb(all[0]), (0x20, 0x40, 0xFF));
    // The other slot keeps its default.
    assert_eq!(cycle_colors(all[1]), vec![BLUE, SECONDARY]);
    assert_eq!(shows[0].palette["primary"], BLUE);
    assert_eq!(shows[0].palette["secondary"], SECONDARY);
}

#[test]
fn an_override_for_an_undeclared_slot_is_ignored() {
    let shows = resolve_shows(parsed(TWO_USES), &overrides(&[("nowhere", BLUE)]));
    let all = effects(&shows[0]);
    assert_eq!(rgb(all[0]), (0xC0, 0x10, 0x20));
    assert!(!shows[0].palette.contains_key("nowhere"));
}

#[test]
fn an_override_recolors_a_slot_in_every_show_that_declares_it() {
    let one = parsed(TWO_USES);
    let other = parse_light_shows(
        "palette { primary: \"green\" }\nshow \"T\" {\n    @0.0\n    a: static, color: palette.primary, duration: 1s\n}\n",
    )
    .unwrap();
    let both: Vec<LightShow> = one.into_iter().chain(other.into_values()).collect();

    // Each file's own default, until an override names the slot.
    let plain = resolve_shows(both.clone(), &HashMap::new());
    assert_eq!(rgb(effects(&plain[0])[0]), (0xC0, 0x10, 0x20));
    assert_eq!(rgb(effects(&plain[1])[0]), (0, 255, 0));

    let recolored = resolve_shows(both, &overrides(&[("primary", BLUE)]));
    assert_eq!(rgb(effects(&recolored[0])[0]), (0x20, 0x40, 0xFF));
    assert_eq!(rgb(effects(&recolored[1])[0]), (0x20, 0x40, 0xFF));
}

#[test]
fn the_declared_slots_of_several_shows_are_listed_once() {
    let one = parsed(TWO_USES);
    let other = parse_light_shows(
        "palette { primary: \"green\", extra: \"white\" }\nshow \"T\" {\n    @0.0\n    a: static, color: palette.extra, duration: 1s\n}\n",
    )
    .unwrap();
    let both: Vec<LightShow> = one.into_iter().chain(other.into_values()).collect();
    let slots = declared_slots(&both);
    let names: Vec<&str> = slots.keys().map(String::as_str).collect();
    assert_eq!(names, ["primary", "secondary", "accent", "extra"]);
    assert_eq!(slots["primary"], PRIMARY, "the first to declare it sets it");
}

#[test]
fn the_timeline_resolves_with_overrides_and_without() {
    use crate::lighting::timeline::LightingTimeline;
    let first_static = |timeline: &LightingTimeline| {
        let (_, effects) = timeline.cue_effects().next().expect("a cue");
        rgb(&effects[0])
    };
    let plain = LightingTimeline::new(parsed(TWO_USES));
    let blue = LightingTimeline::with_palette(parsed(TWO_USES), &overrides(&[("primary", BLUE)]));
    assert_eq!(first_static(&plain), (0xC0, 0x10, 0x20));
    assert_eq!(first_static(&blue), (0x20, 0x40, 0xFF));
}

// ── the document ──────────────────────────────────────────────────────

const WITH_PALETTE: &str = r##"# the look
palette { # slots
    # the main colour
    primary: "#C01020" # red-ish
    secondary: "orange"
    accent: rgb(255, 240, 200)
    # last word
} # done

sequence "hit"(color: palette.accent) {
    @1/1
    a: static, color: $color, duration: 1beat
}

show "S" {
    @1/1
    a: static, color: palette.primary, duration: 1beat
    b: cycle, color: palette.primary, color: "blue", speed: 1.0, duration: 1beat
    sequence "hit"(color: palette.secondary)
}
"##;

fn doc(source: &str) -> crate::lighting::parser::document::Document {
    document(source, None)
        .document
        .unwrap_or_else(|| panic!("{:?}", document(source, None).error))
}

#[test]
fn the_document_holds_the_palette_and_writes_it_back() {
    let read = doc(&format!("{TEMPO}{WITH_PALETTE}"));
    let palette = read.palette.as_ref().expect("a palette");
    let slots: Vec<(&str, &str)> = palette
        .slots
        .iter()
        .map(|s| (s.name.as_str(), s.value.as_str()))
        .collect();
    assert_eq!(
        slots,
        [
            ("primary", "\"#C01020\""),
            ("secondary", "\"orange\""),
            ("accent", "rgb(255, 240, 200)"),
        ]
    );
    assert_eq!(palette.comment.as_deref(), Some("# slots"));
    assert_eq!(palette.close_comment.as_deref(), Some("# done"));
    assert_eq!(
        palette.slots[0].remarks.comments_before,
        Some(vec!["# the main colour".to_string()])
    );
    assert_eq!(
        palette.slots[0].remarks.comment.as_deref(),
        Some("# red-ish")
    );
    assert_eq!(
        palette.comments_after,
        Some(vec!["# last word".to_string()])
    );

    let written = write(&read);
    assert!(written.contains("palette {"), "{written}");
    assert!(
        written.contains("    primary: \"#C01020\" # red-ish"),
        "{written}"
    );
    assert!(written.contains("color: palette.primary"), "{written}");
    assert!(
        written.contains("sequence \"hit\"(color: palette.secondary)"),
        "{written}"
    );
    assert!(
        written.contains("sequence \"hit\"(color: palette.accent)"),
        "{written}"
    );
    // Written once more it is the same, and still parses to the same shows.
    assert_eq!(doc(&written), read);
    assert_eq!(write(&doc(&written)), written);
    assert!(parse_light_shows(&written).is_ok(), "{written}");
}

#[test]
fn the_palette_keeps_its_place_among_the_items() {
    let source = format!("{TEMPO}{WITH_PALETTE}");
    let read = doc(&source);
    let written = write(&read);
    let at = |needle: &str| written.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(at("tempo {") < at("palette {"));
    assert!(at("palette {") < at("sequence \"hit\""));

    // Written after the show it stays there.
    let moved = "show \"S\" {\n    @1/1\n    a: static, color: palette.x, duration: 1beat\n}\n\npalette {\n    x: \"red\"\n}\n";
    let read = doc(moved);
    assert_eq!(write(&read), moved);
}

#[test]
fn an_edit_in_the_editor_keeps_the_palette_and_its_references() {
    use crate::lighting::parser::edit::EffectEdit;
    let read = doc(&format!("{TEMPO}{WITH_PALETTE}"));
    let cue = &read.shows[0].cues[0];
    let cycle = cue.effects[1].clone();
    // The editor sets the cycle's colours, one of them a slot.
    let edited = cycle.edited(&EffectEdit::Colors {
        colors: vec!["palette.secondary".to_string(), "green".to_string()],
    });
    assert_eq!(
        edited.effect.view().colors,
        vec!["palette.secondary", "green"]
    );
    let mut next = read.clone();
    next.shows[0].cues[0].effects[1] = edited;
    let written = write(&next);
    assert!(
        written.contains("color: palette.secondary, color: \"green\""),
        "{written}"
    );
    assert!(
        written.contains("    primary: \"#C01020\" # red-ish"),
        "{written}"
    );
    let shows = parse_light_shows(&written).unwrap();
    let all = effects(show(&shows));
    assert_eq!(cycle_colors(all[1]), vec![SECONDARY, Color::new(0, 255, 0)]);
}

#[test]
fn a_second_palette_block_is_a_parse_error_for_the_document_too() {
    let result = document(
        "palette { a: \"red\" }\npalette { b: \"red\" }\nshow \"S\" {\n    @1/1\n    a: static, color: palette.a, duration: 1beat\n}\n",
        None,
    );
    assert!(result.document.is_none());
    assert!(result.error.unwrap().contains("second palette block"));
}

#[test]
fn a_sequence_in_an_included_file_can_use_the_included_palette() {
    use crate::lighting::parser::Sources;
    let mut sources = Sources::none();
    sources.add(
        "/songs/common.light",
        "palette { glow: \"magenta\" }\nsequence \"flash\" {\n    @0.0\n    a: static, color: palette.glow, duration: 1s\n}\n"
            .to_string(),
    );
    let shows = parse_light_file_with(
        "include \"common.light\"\nshow \"S\" {\n    @0.0\n    sequence \"flash\"\n}\n",
        Some(std::path::Path::new("/songs/main.light")),
        None,
        None,
        &sources,
    )
    .unwrap();
    let played = show(&shows);
    assert_eq!(rgb(effects(played)[0]), (255, 0, 255));
    assert_eq!(played.palette["glow"], Color::new(255, 0, 255));

    // The including file's colour wins for a slot both declare.
    let shows = parse_light_file_with(
        "include \"common.light\"\npalette { glow: \"cyan\" }\nshow \"S\" {\n    @0.0\n    sequence \"flash\"\n}\n",
        Some(std::path::Path::new("/songs/main.light")),
        None,
        None,
        &sources,
    )
    .unwrap();
    assert_eq!(rgb(effects(show(&shows))[0]), (0, 255, 255));
}

#[test]
fn an_inline_loop_names_the_files_palette_slots() {
    let shows = parse(&format!(
        "{PALETTE}\nshow \"S\" {{\n    @1/1\n    loop {{\n        @1/1\n        a: static, color: palette.primary, duration: 1beat\n    }} repeats: 2\n}}\n"
    ))
    .unwrap();
    let found = effects(show(&shows));
    assert_eq!(found.len(), 2, "one per pass");
    for effect in found {
        assert_eq!(rgb(effect), (PRIMARY.r, PRIMARY.g, PRIMARY.b));
    }

    let resolved = crate::lighting::parser::resolve_shows(
        vec![show(&shows).clone()],
        &overrides(&[("primary", BLUE)]),
    );
    for effect in resolved[0].cues.iter().flat_map(|c| c.effects.iter()) {
        assert_eq!(
            rgb(effect),
            (BLUE.r, BLUE.g, BLUE.b),
            "overridden in the loop"
        );
    }

    let err = parse(
        "show \"S\" {\n    @1/1\n    loop {\n        @1/1\n        a: static, color: palette.nope, duration: 1beat\n    } repeats: 2\n}\n",
    )
    .unwrap_err();
    assert!(err.contains("palette slot 'nope' is not declared"), "{err}");
}
