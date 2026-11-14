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
//! The `lighting_dsl_reference` text: the prose of `dsl_reference.md`, its
//! `{{name}}` slots filled from the parser's own tables, so the effects,
//! their parameters and defaults, and every keyword list are the grammar's.

use crate::config::length::UNITS;
use crate::lighting::effects::{LayerSet, NAMED_COLORS};
use crate::lighting::parser::document::EffectKind;
use crate::lighting::parser::edit::vocabulary;
use crate::lighting::parser::words::{
    effect_defaults, LayerCommandParam, CHASE_TRANSITIONS, COMMON_PARAMS, CYCLE_TRANSITIONS,
    PER_KEYWORDS,
};

/// The reference's prose, with a `{{name}}` slot for each table.
const TEMPLATE: &str = include_str!("dsl_reference.md");

/// What the table cannot say about an effect. One per effect, so a new
/// effect cannot reach the reference without one.
fn effect_note(kind: EffectKind) -> &'static str {
    match kind {
        EffectKind::Static => {
            "Holds a colour and levels. `dimmer` or `intensity` is the level; `red`, `green`, \
             `blue`, `white` set one channel each. Any other name with a level sets that channel \
             on the fixtures that have it."
        }
        EffectKind::Cycle => {
            "Steps through its colours: give `color:` once per colour. `transition: hue` fades \
             around the colour wheel: use it when consecutive colours are far apart (yellow → \
             blue, red → cyan), whose plain fade passes through grey."
        }
        EffectKind::Strobe => {
            "Flashes at `frequency` Hz (`rate` is the same). Takes no color or intensity: it \
             gates whatever is beneath it."
        }
        EffectKind::Pulse => "Sinusoidal pulse. `intensity` is `pulse_amplitude` by another name.",
        EffectKind::Chase => {
            "A moving brightness mask over the layers beneath — needs a color bed under it. With \
             `blend_mode: multiply` (or `replace`) it gates the bed: one fixture lit, the rest \
             dark. With `screen` or `add` it lifts: the active fixture brightens toward white and \
             the others keep the bed, so the stage stays lit."
        }
        EffectKind::Dimmer => {
            "A ramp from `start_level` (or `start`) to `end_level` (or `end`) along `curve`. The \
             one effect whose `duration` has a default; it ignores `up_time`, `hold_time` and \
             `down_time`."
        }
        EffectKind::Rainbow => "Hue sweep.",
        EffectKind::Move => {
            "Aims movers at a venue focus point (`focus`, or `to`) or at `pan`/`tilt` angles \
             (`45deg`), starting from `from` (or `from_pan`/`from_tilt`) or from wherever each \
             head is. `duration` is the travel time and must be written (`hold_time` does not \
             stand in for it); a head that has arrived holds its pose until the next `move` on \
             it or a `clear`."
        }
    }
}

/// The effects table: each effect, the parameters it reads, their
/// defaults, and its note.
fn effects_table(layers: &LayerSet) -> String {
    let mut out = String::from(
        "| Effect | Parameters it reads | Defaults | Notes |\n\
         |--------|---------------------|----------|-------|\n",
    );
    for effect in vocabulary(layers).effects {
        let params = effect
            .params
            .iter()
            .map(|p| format!("`{p}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let defaults = effect_defaults(effect.kind);
        let defaults = if defaults.is_empty() {
            "—".to_string()
        } else {
            defaults
                .iter()
                .map(|(name, value)| format!("`{name}: {value}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "| `{}` | {params} | {defaults} | {} |\n",
            effect.kind.keyword(),
            effect_note(effect.kind)
        ));
    }
    out
}

/// `words` as the reference lists keywords: each in backticks, `|` between.
fn alternatives<'a>(words: impl IntoIterator<Item = &'a str>) -> String {
    words
        .into_iter()
        .map(|w| format!("`{w}`"))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// `words` in backticks, separated by commas.
fn listed<'a>(words: impl IntoIterator<Item = &'a str>) -> String {
    words
        .into_iter()
        .map(|w| format!("`{w}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What slot `name` holds, or `None` for no such slot.
fn slot(name: &str, layers: &LayerSet) -> Option<String> {
    let v = vocabulary(layers);
    Some(match name {
        "effects" => effects_table(layers),
        "common_params" => listed(COMMON_PARAMS),
        "units" => listed(UNITS.iter().flat_map(|(_, plural, one)| {
            if plural == one {
                vec![*plural]
            } else {
                vec![*plural, *one]
            }
        })),
        "named_colors" => listed(NAMED_COLORS.iter().map(|(name, _)| *name)),
        "cycle_directions" => alternatives(v.cycle_directions),
        "chase_directions" => alternatives(v.chase_directions),
        "chase_patterns" => alternatives(v.chase_patterns),
        "cycle_transitions" => alternatives(CYCLE_TRANSITIONS.iter().map(|(_, w)| *w)),
        "chase_transitions" => alternatives(CHASE_TRANSITIONS.iter().map(|(_, w)| *w)),
        "curves" => alternatives(v.curves),
        "easings" => alternatives(v.easings),
        "layers" => alternatives(v.layers.iter().map(String::as_str)),
        "blend_modes" => alternatives(v.blend_modes),
        "layer_commands" => listed(v.layer_commands.iter().map(|c| c.keyword())),
        "layer_command_params" => listed(LayerCommandParam::ALL.iter().map(|(_, w)| *w)),
        "per" => listed(PER_KEYWORDS.iter().map(|(_, w)| *w)),
        "color_effects" => {
            let readers: Vec<&str> = v
                .effects
                .iter()
                .filter(|e| e.params.contains(&"color"))
                .map(|e| e.kind.keyword())
                .collect();
            let mut words = listed(readers);
            // `a`, `b` and `c`.
            if let Some(last) = words.rfind(", ") {
                words.replace_range(last..last + 2, " and ");
            }
            words
        }
        _ => return None,
    })
}

/// The reference, every slot filled.
pub(super) fn dsl_reference(layers: &LayerSet) -> String {
    let mut out = String::with_capacity(TEMPLATE.len() * 2);
    let mut rest = TEMPLATE;
    while let Some(start) = rest.find("{{") {
        let end = rest[start..]
            .find("}}")
            .map(|e| start + e)
            .expect("every slot is closed");
        let name = &rest[start + 2..end];
        out.push_str(&rest[..start]);
        out.push_str(&slot(name, layers).unwrap_or_else(|| panic!("no slot named {name}")));
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_is_filled() {
        let text = dsl_reference(LayerSet::standard());
        assert!(!text.contains("{{") && !text.contains("}}"));
    }

    #[test]
    fn every_effect_and_keyword_is_in_the_reference() {
        let text = dsl_reference(LayerSet::standard());
        let v = vocabulary(LayerSet::standard());
        for effect in &v.effects {
            let row = text
                .lines()
                .find(|l| l.starts_with(&format!("| `{}` |", effect.kind.keyword())))
                .unwrap_or_else(|| panic!("no row for {}", effect.kind.keyword()));
            for p in &effect.params {
                assert!(row.contains(&format!("`{p}`")), "{p} missing from {row}");
            }
        }
        let words = v
            .layers
            .iter()
            .map(String::as_str)
            .chain(v.blend_modes.iter().copied())
            .chain(v.curves.iter().copied())
            .chain(v.easings.iter().copied())
            .chain(v.cycle_directions.iter().copied())
            .chain(v.chase_directions.iter().copied())
            .chain(v.chase_patterns.iter().copied())
            .chain(COMMON_PARAMS.iter().copied())
            .chain(CYCLE_TRANSITIONS.iter().map(|(_, w)| *w));
        for word in words {
            assert!(text.contains(&format!("`{word}`")), "{word} missing");
        }
    }

    #[test]
    fn the_word_lists_are_the_parsers() {
        let text = dsl_reference(LayerSet::standard());
        let v = vocabulary(LayerSet::standard());
        let units = UNITS.iter().flat_map(|(_, plural, one)| [*plural, *one]);
        let colors = NAMED_COLORS.iter().map(|(name, _)| *name);
        let commands = v.layer_commands.iter().map(|c| c.keyword());
        let per = PER_KEYWORDS.iter().map(|(_, w)| *w);
        for word in units.chain(colors).chain(commands).chain(per) {
            assert!(text.contains(&format!("`{word}`")), "{word} missing");
        }
        // The layer commands' parameters, read off their own sentence: the
        // words are common enough to appear elsewhere in the reference.
        let params = text
            .split_once("parenthesised parameters (")
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(params, _)| params)
            .expect("the layer commands' parameters");
        let mut listed: Vec<&str> = params.split('`').skip(1).step_by(2).collect();
        let mut want: Vec<&str> = LayerCommandParam::ALL.iter().map(|(_, w)| *w).collect();
        listed.sort();
        want.sort();
        assert_eq!(listed, want, "{params}");
        // The effects that read `color`, and no other, are named as its readers.
        let readers = text
            .lines()
            .find_map(|l| l.split_once("Only ").map(|(_, rest)| rest))
            .and_then(|rest| rest.split_once(" read it"))
            .map(|(readers, _)| readers)
            .expect("the color line");
        for effect in &v.effects {
            assert_eq!(
                readers.contains(&format!("`{}`", effect.kind.keyword())),
                effect.params.contains(&"color"),
                "{readers}"
            );
        }
    }

    #[test]
    fn the_dimmer_states_its_defaults() {
        let text = dsl_reference(LayerSet::standard());
        let row = text
            .lines()
            .find(|l| l.starts_with("| `dimmer` |"))
            .unwrap();
        for default in [
            "`start_level: 0%`",
            "`end_level: 100%`",
            "`duration: 1s`",
            "`curve: linear`",
        ] {
            assert!(row.contains(default), "{default} missing from {row}");
        }
        let row = text
            .lines()
            .find(|l| l.starts_with("| `static` |"))
            .unwrap();
        assert!(row.contains("`white`"));
    }
}
