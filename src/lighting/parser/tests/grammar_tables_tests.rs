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
//! Each keyword list the grammar writes out is its Rust table: the words
//! a rule admits are read off `grammar.pest` itself and compared with the
//! table the parser reads them by, so neither can gain a word alone.

use pest::Parser;

use crate::lighting::effects::NAMED_COLORS;
use crate::lighting::fixture_data::StrobeCurve;
use crate::lighting::parser::effect_parse::{
    CHASE_DIRECTIONS, CHASE_PATTERNS, CHASE_TRANSITIONS, CYCLE_DIRECTIONS, CYCLE_TRANSITIONS,
};
use crate::lighting::parser::grammar::{LightingParser, Rule};
use crate::lighting::parser::types::{
    LayerCommandParam, EFFECT_KEYWORDS, LAYER_COMMAND_KEYWORDS, SEQUENCE_LOOPS,
};
use crate::lighting::parser::utils::{BLEND_KEYWORDS, LAYER_KEYWORDS, UNITS};

/// The quoted words of the grammar rule `name`, which may span lines.
fn grammar_words(name: &str) -> Vec<String> {
    let grammar = include_str!("../../grammar.pest");
    let start = grammar
        .lines()
        .position(|line| line.starts_with(&format!("{name} =")))
        .unwrap_or_else(|| panic!("no rule {name}"));
    let mut body = String::new();
    for line in grammar.lines().skip(start) {
        body.push_str(line);
        if line.trim_end().ends_with('}') {
            break;
        }
    }
    // A quoted quote (`"\""`) is no word, and would pair the quotes off.
    body.replace(r#""\"""#, "")
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(str::to_string)
        .collect::<Vec<_>>()
}

/// The words of `table`, the second of each entry.
fn words<T>(table: &[(T, &'static str)]) -> Vec<&'static str> {
    table.iter().map(|(_, w)| *w).collect()
}

/// `rule` (named `name` in the grammar) admits `table`'s words and no
/// other: the same set, each read whole by the rule, and a word not in it
/// refused. `wrap` writes a word as the rule reads it (a quoted colour).
fn assert_rule_is(rule: Rule, name: &str, table: &[&str], wrap: fn(&str) -> String) {
    let mut written = grammar_words(name);
    let mut expected: Vec<String> = table.iter().map(|w| w.to_string()).collect();
    written.sort();
    written.dedup();
    expected.sort();
    expected.dedup();
    assert_eq!(written, expected, "rule {name}");
    for word in &expected {
        let text = wrap(word);
        let pair = LightingParser::parse(rule, &text)
            .unwrap_or_else(|e| panic!("{name} refuses {text}: {e}"))
            .next()
            .unwrap();
        assert_eq!(pair.as_str(), text, "{name} reads {text} whole");
    }
    let stranger = wrap("zzz_not_a_keyword");
    assert!(
        LightingParser::parse(rule, &stranger).is_err(),
        "{name} admits {stranger}"
    );
}

fn bare(word: &str) -> String {
    word.to_string()
}

#[test]
fn the_grammar_names_exactly_the_named_colors() {
    let named: Vec<&str> = NAMED_COLORS.iter().map(|(name, _)| *name).collect();
    assert_rule_is(Rule::named_color, "named_color", &named, |w| {
        format!("\"{w}\"")
    });
}

#[test]
fn the_grammar_names_exactly_the_effects() {
    assert_rule_is(
        Rule::effect_type,
        "effect_type",
        &words(&EFFECT_KEYWORDS),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_directions() {
    let mut both = words(&CYCLE_DIRECTIONS);
    both.extend(words(&CHASE_DIRECTIONS));
    assert_rule_is(
        Rule::direction_parameter,
        "direction_parameter",
        &both,
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_chase_patterns() {
    assert_rule_is(
        Rule::chase_pattern_parameter,
        "chase_pattern_parameter",
        &words(&CHASE_PATTERNS),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_sequence_loops() {
    assert_rule_is(
        Rule::loop_parameter,
        "loop_parameter",
        &words(&SEQUENCE_LOOPS),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_transitions() {
    // A chase's transitions are a cycle's, less `hue`.
    let chase = words(&CHASE_TRANSITIONS);
    assert!(chase.iter().all(|w| words(&CYCLE_TRANSITIONS).contains(w)));
    assert_rule_is(
        Rule::transition_parameter,
        "transition_parameter",
        &words(&CYCLE_TRANSITIONS),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_layers() {
    assert_rule_is(
        Rule::layer_parameter,
        "layer_parameter",
        &words(&LAYER_KEYWORDS),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_blend_modes() {
    assert_rule_is(
        Rule::blend_mode_parameter,
        "blend_mode_parameter",
        &words(&BLEND_KEYWORDS),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_time_units() {
    let spellings: Vec<&str> = UNITS
        .iter()
        .flat_map(|(_, plural, one)| [*plural, *one])
        .collect();
    assert_rule_is(Rule::time_unit, "time_unit", &spellings, bare);
}

#[test]
fn the_grammar_names_exactly_the_layer_commands() {
    assert_rule_is(
        Rule::layer_command_type,
        "layer_command_type",
        &words(&LAYER_COMMAND_KEYWORDS),
        bare,
    );
    assert_rule_is(
        Rule::layer_command_param_name,
        "layer_command_param_name",
        &words(&LayerCommandParam::ALL),
        bare,
    );
}

#[test]
fn the_grammar_names_exactly_the_strobe_curves() {
    let keywords: Vec<&str> = <StrobeCurve as strum::VariantArray>::VARIANTS
        .iter()
        .map(|c| c.keyword())
        .collect();
    assert_rule_is(
        Rule::strobe_curve_value,
        "strobe_curve_value",
        &keywords,
        bare,
    );
}

/// The movement limits have no table: each is a field the parser sets by
/// name. Each name the grammar admits sets its own limit.
#[test]
fn the_grammar_names_exactly_the_movement_limits() {
    use crate::lighting::parser::parse_fixture_types;
    let names = grammar_words("movement_param_name");
    assert_eq!(names, ["max_pan_speed", "max_tilt_speed"]);
    for name in names {
        let source = format!(
            "fixture_type \"M\" {{\n  channel \"pan\" @ 1\n  channel \"tilt\" @ 2\n  movement {{ {name}: 90deg/s }}\n}}"
        );
        let types = parse_fixture_types(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
        let movement = types["M"].movement();
        let set = match name.as_str() {
            "max_pan_speed" => movement.max_pan_speed,
            _ => movement.max_tilt_speed,
        };
        assert_eq!(set, Some(90.0), "{name}");
    }
}
