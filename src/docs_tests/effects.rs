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
//! `docs/src/lighting/effects.md` against the parser's tables: each
//! effect's parameters and defaults, and every keyword list.

use super::{stated, Stated};
use crate::config::length::UNITS;
use crate::lighting::effects::{CycleTransition, NAMED_COLORS};
use crate::lighting::parser::document::EffectKind;
use crate::lighting::parser::edit::vocabulary;
use crate::lighting::parser::words::{
    effect_defaults, CHASE_TRANSITIONS, COMMON_PARAMS, CYCLE_TRANSITIONS,
};

const EFFECTS_MD: &str = include_str!("../../docs/src/lighting/effects.md");

/// The text under the heading line `heading`, to the next heading of the
/// same or a higher level.
fn section<'a>(markdown: &'a str, heading: &str) -> &'a str {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let start = markdown
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("no heading {heading}"))
        + heading.len()
        + 2;
    let rest = &markdown[start..];
    let end = rest
        .lines()
        .scan(0, |at, line| {
            let here = *at;
            *at += line.len() + 1;
            Some((here, line))
        })
        .find(|(_, line)| {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            hashes > 0 && hashes <= level && line[hashes..].starts_with(' ')
        })
        .map(|(at, _)| at)
        .unwrap_or(rest.len());
    &rest[..end]
}

/// The backticked words of `text`.
fn ticked(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// Where a bullet's names end: its first `:` outside backticks.
fn head_end(bullet: &str) -> usize {
    let mut ticks = 0;
    for (i, c) in bullet.char_indices() {
        match c {
            '`' => ticks += 1,
            ':' if ticks % 2 == 0 => return i,
            _ => {}
        }
    }
    0
}

/// Each `` - `name`…: description`` bullet of `text` (continuation lines
/// joined), as (the names it documents, the whole bullet).
fn bullets(text: &str) -> Vec<(Vec<String>, String)> {
    let mut joined: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        if line.starts_with("- `") {
            joined.push(line.to_string());
            open = true;
        } else if open && line.starts_with("  ") {
            let last = joined.last_mut().unwrap();
            last.push(' ');
            last.push_str(line.trim());
        } else {
            open = false;
        }
    }
    joined
        .into_iter()
        .map(|bullet| {
            let names = ticked(&bullet[..head_end(&bullet)])
                .into_iter()
                .map(str::to_string)
                .collect();
            (names, bullet)
        })
        .collect()
}

/// Whether `names` holds `name`.
fn has(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n == name)
}

/// The defaults a bullet states: `Default X`, `Defaults to X`, and a
/// `` `word` (… default …)``.
fn stated_defaults(bullet: &str) -> Vec<Stated> {
    let mut out = Vec::new();
    let lower = bullet.to_lowercase();
    for marker in ["defaults to ", "default "] {
        for (i, _) in lower.match_indices(marker) {
            let word = bullet[i + marker.len()..].split_whitespace().next();
            if let Some(word) = word {
                out.push(stated(word));
            }
        }
    }
    for (open, _) in bullet.match_indices(" (") {
        let inside = &bullet[open..];
        let inside = &inside[..inside.find(')').unwrap_or(inside.len())];
        let before = &bullet[..open];
        if inside.contains("default") && before.ends_with('`') {
            if let Some(word) = before.rsplit('`').nth(1) {
                out.push(stated(word));
            }
        }
    }
    out
}

/// Each effect's section heading.
fn heading(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Static => "### Static Effect",
        EffectKind::Cycle => "### Color Cycle Effect",
        EffectKind::Strobe => "### Strobe Effect",
        EffectKind::Pulse => "### Pulse Effect",
        EffectKind::Chase => "### Chase Effect",
        EffectKind::Dimmer => "### Dimmer Effect",
        EffectKind::Rainbow => "### Rainbow Effect",
        EffectKind::Move => "### Move Effect",
    }
}

#[test]
fn every_effect_has_a_section_and_no_other() {
    let types = section(EFFECTS_MD, "## Effect Types");
    let written = types.lines().filter(|l| l.starts_with("### ")).count();
    assert_eq!(written, vocabulary().effects.len());
    for effect in vocabulary().effects {
        assert!(types.contains(&format!("\n{}\n", heading(effect.kind))));
    }
}

#[test]
fn each_effect_lists_the_parameters_it_reads_and_no_other() {
    for effect in vocabulary().effects {
        let text = section(EFFECTS_MD, heading(effect.kind));
        let keyword = effect.kind.keyword();
        for param in &effect.params {
            assert!(
                text.contains(&format!("`{param}`")),
                "{keyword}: `{param}` is not documented"
            );
        }
        for (names, bullet) in bullets(text) {
            for name in names {
                assert!(
                    effect.params.contains(&name.as_str()),
                    "{keyword}: `{name}` is documented but not read ({bullet})"
                );
            }
        }
    }
}

#[test]
fn each_effect_states_its_defaults_and_no_other() {
    for effect in vocabulary().effects {
        let text = section(EFFECTS_MD, heading(effect.kind));
        let keyword = effect.kind.keyword();
        let defaults = effect_defaults(effect.kind);
        for (names, bullet) in bullets(text) {
            let said = stated_defaults(&bullet);
            let want: Vec<Stated> = defaults
                .iter()
                .filter(|(name, _)| has(&names, name))
                .map(|(_, value)| stated(value))
                .collect();
            assert_eq!(said, want, "{keyword}: {bullet}");
            if has(&names, "duration") {
                let required = !defaults.iter().any(|(name, _)| *name == "duration");
                assert_eq!(
                    bullet.contains("**Required"),
                    required,
                    "{keyword}: {bullet}"
                );
            }
        }
        for (name, _) in &defaults {
            assert!(
                bullets(text).iter().any(|(names, _)| has(names, name)),
                "{keyword}: no bullet for `{name}`"
            );
        }
    }
}

/// The bullet of `effect`'s section documenting `name`.
fn bullet_for(kind: EffectKind, name: &str) -> String {
    bullets(section(EFFECTS_MD, heading(kind)))
        .into_iter()
        .find(|(names, _)| has(names, name))
        .map(|(_, b)| b)
        .unwrap_or_else(|| panic!("{}: no bullet for {name}", kind.keyword()))
}

/// The backticked words of `bullet` after its names: the values it lists.
fn values(bullet: &str) -> Vec<String> {
    let body = &bullet[head_end(bullet)..];
    let mut out: Vec<String> = ticked(body).into_iter().map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}

fn sorted(words: impl IntoIterator<Item = &'static str>) -> Vec<String> {
    let mut out: Vec<String> = words.into_iter().map(str::to_string).collect();
    out.sort();
    out.dedup();
    out
}

#[test]
fn the_keyword_lists_are_the_parsers() {
    let v = vocabulary();
    let words = |table: &[(CycleTransition, &'static str)]| -> Vec<&'static str> {
        table.iter().map(|(_, w)| *w).collect()
    };
    let cases: Vec<(EffectKind, &str, Vec<String>)> = vec![
        (
            EffectKind::Cycle,
            "direction",
            sorted(v.cycle_directions.clone()),
        ),
        (
            EffectKind::Cycle,
            "transition",
            sorted(words(&CYCLE_TRANSITIONS)),
        ),
        (
            EffectKind::Chase,
            "pattern",
            sorted(v.chase_patterns.clone()),
        ),
        (
            EffectKind::Chase,
            "transition",
            sorted(words(&CHASE_TRANSITIONS)),
        ),
        (EffectKind::Dimmer, "curve", sorted(v.curves.clone())),
        (EffectKind::Move, "easing", sorted(v.easings.clone())),
    ];
    for (kind, name, want) in cases {
        assert_eq!(
            values(&bullet_for(kind, name)),
            want,
            "{} {name}",
            kind.keyword()
        );
    }
    // The chase directions' bullet goes on to say what they mean.
    let chase = bullet_for(EffectKind::Chase, "direction");
    let listed = &chase[..chase.find(".").unwrap()];
    assert_eq!(values(listed), sorted(v.chase_directions.clone()));
}

#[test]
fn the_common_parameters_are_the_parsers() {
    let v = vocabulary();
    let text = section(EFFECTS_MD, "## Common Effect Parameters");
    let mut documented: Vec<String> = bullets(text).into_iter().flat_map(|(n, _)| n).collect();
    documented.sort();
    assert_eq!(documented, sorted(COMMON_PARAMS));
    let common = |name: &str| {
        bullets(text)
            .into_iter()
            .find(|(n, _)| has(n, name))
            .map(|(_, b)| b)
            .unwrap()
    };
    let layers = common("layer");
    for layer in &v.layers {
        assert!(layers.contains(&format!("`{layer}`")), "{layer}");
    }
    let blends = common("blend_mode");
    let listed = &blends[..blends.find("`replace` writes").unwrap()];
    assert_eq!(values(listed), sorted(v.blend_modes.clone()));
}

#[test]
fn the_units_and_named_colors_are_the_parsers() {
    let values_paragraph = EFFECTS_MD
        .split("\n\n")
        .find(|p| p.starts_with("**Values.**"))
        .unwrap();
    let mut units: Vec<String> = ticked(values_paragraph)
        .into_iter()
        .filter_map(|w| {
            let unit = w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
            let measured = unit.len() < w.len() && !unit.is_empty();
            (measured && unit.chars().all(|c| c.is_ascii_alphabetic())).then(|| unit.to_string())
        })
        .collect();
    units.sort();
    units.dedup();
    assert_eq!(
        units,
        sorted(UNITS.iter().flat_map(|(_, plural, one)| [*plural, *one]))
    );
    let colors = EFFECTS_MD
        .split("\n\n")
        .find(|p| p.starts_with("**Colors.**"))
        .unwrap();
    let named: Vec<&str> = ticked(colors)
        .into_iter()
        .filter_map(|w| w.strip_prefix('"').and_then(|w| w.strip_suffix('"')))
        .collect();
    assert_eq!(
        sorted_owned(&named),
        sorted(NAMED_COLORS.iter().map(|(n, _)| *n))
    );
}

fn sorted_owned(words: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    out.sort();
    out.dedup();
    out
}
