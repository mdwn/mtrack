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
//! The editor's edits of a [`super::document::Document`] that write DSL
//! text into it -- a quoted focus point, an angle in degrees, a colour, a
//! loop count -- or read it back for a form (a value without its quotes),
//! so the editor holds and changes the document without writing the
//! grammar itself. Each edit touches the text it edits alone: a parameter
//! set keeps its place and comments, one removed leaves its comments where
//! it was (see [`super::writer`]'s `dropped`).

use std::collections::HashMap;

use pest::Parser;
use serde::{Deserialize, Serialize};

use super::document::DurationUnit;
use super::document::{
    CueEffect, EffectKind, EffectParams, LayerCommandType, Param, Remarks, SequenceRef,
};
use super::effect_parse::{
    CHASE_DIRECTIONS, CHASE_PATTERNS, COMMON_PARAMS, CYCLE_DIRECTIONS, DIMMER_CURVES, EASINGS,
    MOVE_ALIASES,
};
use super::grammar::{LightingParser, Rule};
use super::show::parse_sequence_loop_param;
use super::types::{SequenceLoop, EFFECT_KEYWORDS, LAYER_COMMAND_KEYWORDS, SEQUENCE_LOOPS};
use super::utils::{unquoted, BLEND_KEYWORDS, LAYER_KEYWORDS};
use super::writer::{dropped, length_text, number};

/// An edit of an effect line's parameters, as a form makes it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EffectEdit {
    /// `name` set to `value` as typed (its last one replaced where it
    /// was, or a new one added), or removed when `value` is empty.
    Set {
        name: String,
        #[serde(default)]
        #[cfg_attr(feature = "tsify", tsify(optional))]
        value: Option<String>,
    },
    /// Every parameter of these names removed.
    Remove { names: Vec<String> },
    /// These `color:` values and no other, where the first colour was.
    Colors { colors: Vec<String> },
    /// A `move`'s target or start: a focus point or angles, setting one
    /// clearing the other.
    Move {
        key: MoveParam,
        #[serde(default)]
        #[cfg_attr(feature = "tsify", tsify(optional))]
        value: Option<String>,
    },
    /// The effect turned into `kind`, keeping only the parameters `kind`
    /// reads and those every effect takes.
    Kind { kind: EffectKind },
}

/// The words an effect line's form offers, off the parser's own tables:
/// each effect and the parameters it reads, and the values each keyword
/// parameter takes.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(into_wasm_abi))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Vocabulary {
    pub effects: Vec<EffectWords>,
    pub layers: Vec<&'static str>,
    pub blend_modes: Vec<&'static str>,
    pub curves: Vec<&'static str>,
    pub easings: Vec<&'static str>,
    pub cycle_directions: Vec<&'static str>,
    pub chase_directions: Vec<&'static str>,
    pub chase_patterns: Vec<&'static str>,
    pub layer_commands: Vec<LayerCommandType>,
    pub loops: Vec<&'static str>,
}

/// An effect and the parameters it reads (`EffectKind::params`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EffectWords {
    pub kind: EffectKind,
    pub params: Vec<&'static str>,
}

/// The [`Vocabulary`] of the grammar.
pub fn vocabulary() -> Vocabulary {
    fn words<T>(table: &[(T, &'static str)]) -> Vec<&'static str> {
        table.iter().map(|(_, word)| *word).collect()
    }
    Vocabulary {
        effects: EFFECT_KEYWORDS
            .iter()
            .map(|(kind, _)| EffectWords {
                kind: *kind,
                params: kind.params().to_vec(),
            })
            .collect(),
        layers: words(&LAYER_KEYWORDS),
        blend_modes: words(&BLEND_KEYWORDS),
        curves: words(&DIMMER_CURVES),
        easings: words(&EASINGS),
        cycle_directions: words(&CYCLE_DIRECTIONS),
        chase_directions: words(&CHASE_DIRECTIONS),
        chase_patterns: words(&CHASE_PATTERNS),
        layer_commands: LAYER_COMMAND_KEYWORDS.iter().map(|(c, _)| *c).collect(),
        loops: words(&SEQUENCE_LOOPS),
    }
}

pub use super::effect_parse::MoveParam;

/// An effect's parameters as a form shows them: the last value of each
/// name as written (a `to:` read as `focus`, a focus point and a `from:`
/// without their quotes), and its colours without theirs.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EffectView {
    pub values: HashMap<String, String>,
    pub colors: Vec<String>,
}

impl CueEffect {
    /// The effect with `edit` made.
    pub fn edited(&self, edit: &EffectEdit) -> CueEffect {
        match edit {
            EffectEdit::Set { name, value } => self.with_param(name, value.as_deref()),
            EffectEdit::Remove { names } => self.without_params(names),
            EffectEdit::Colors { colors } => self.with_colors(colors),
            EffectEdit::Kind { kind } => {
                let keeps = |p: &Param| {
                    kind.params().contains(&p.name.as_str())
                        || COMMON_PARAMS.contains(&p.name.as_str())
                };
                let mut next = self.with_kept(self.effect.params.clone(), |p, _| !keeps(p));
                next.effect.kind = *kind;
                next
            }
            EffectEdit::Move { key, value } => {
                let value = value.as_deref().filter(|v| !v.is_empty());
                let next = match value {
                    Some(_) => self.without_params(&key.excludes()),
                    None => self.clone(),
                };
                let written = value.map(|v| {
                    if key.is_point() {
                        quoted(v)
                    } else {
                        with_deg(v)
                    }
                });
                next.with_param(key.name(), written.as_deref())
            }
        }
    }

    /// The effect with `name` set to `value` (its last one replaced, or a
    /// new one added), or without it when `value` is empty or `None`.
    pub fn with_param(&self, name: &str, value: Option<&str>) -> CueEffect {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            return self.without_params(&[name]);
        };
        let mut next = self.clone();
        let params = &mut next.effect.params;
        // Its text alone changes: the comments around it stay.
        match params.iter().rposition(|p| p.name == name) {
            Some(last) => {
                params[last].value = value.to_string();
                params[last].is_ref = None;
            }
            None => params.push(Param {
                name: name.to_string(),
                value: value.to_string(),
                is_ref: None,
                remarks: Remarks::default(),
            }),
        }
        next
    }

    /// The effect without any of `names`.
    pub fn without_params<S: AsRef<str>>(&self, names: &[S]) -> CueEffect {
        self.with_kept(self.effect.params.clone(), |p, _| {
            names.iter().any(|n| n.as_ref() == p.name)
        })
    }

    /// The effect with `params`, those `drop` picks removed and their
    /// comments kept where they were (see `dropped`); with no parameter
    /// left, as comment lines after the effect's line.
    fn with_kept(&self, params: Vec<Param>, drop: impl Fn(&Param, usize) -> bool) -> CueEffect {
        let (kept, orphans) = dropped(params, |p| &mut p.remarks, drop);
        let mut next = self.clone();
        next.effect.params = kept;
        if !orphans.is_empty() {
            let mut after = next.comments_after.take().unwrap_or_default();
            after.extend(orphans);
            next.comments_after = Some(after);
        }
        next
    }

    /// The effect with these `color:` values and no other, where the first
    /// colour was (or at the front).
    pub fn with_colors(&self, colors: &[String]) -> CueEffect {
        let all = &self.effect.params;
        let at = all.iter().position(|p| p.name == "color").unwrap_or(0);
        let rest: Vec<Param> = all.iter().filter(|p| p.name != "color").cloned().collect();
        let was: Vec<Param> = all.iter().filter(|p| p.name == "color").cloned().collect();
        // Each colour keeps the comments of the one it replaces.
        let entries = colors.iter().enumerate().map(|(i, c)| Param {
            name: "color".to_string(),
            value: quoted(c),
            is_ref: None,
            remarks: was.get(i).map(|w| w.remarks.clone()).unwrap_or_default(),
        });
        // A colour no longer there leaves its comments where it was.
        let gone = was.len().saturating_sub(colors.len());
        let at = at.min(rest.len());
        let mut params: Vec<Param> = rest[..at].to_vec();
        params.extend(entries);
        params.extend(was.iter().skip(colors.len()).cloned());
        params.extend(rest[at..].iter().cloned());
        let from = at + colors.len();
        self.with_kept(params, |_, i| i >= from && i < from + gone)
    }
}

impl EffectParams {
    /// The parameters as a form shows them (see [`EffectView`]).
    pub fn view(&self) -> EffectView {
        let mut values: HashMap<String, String> = HashMap::new();
        for p in &self.params {
            values.insert(p.name.clone(), p.value.clone());
        }
        for (alias, param) in MOVE_ALIASES {
            if !values.contains_key(param.name()) {
                if let Some(value) = values.get(alias).cloned() {
                    values.insert(param.name().to_string(), value);
                }
            }
        }
        for param in MoveParam::points() {
            let shown = values.get(param.name()).map(|v| unquoted(v).to_string());
            values.insert(param.name().to_string(), shown.unwrap_or_default());
        }
        let colors = self
            .params
            .iter()
            .filter(|p| p.name == "color")
            .map(|p| unquoted(&p.value).to_string())
            .collect();
        EffectView { values, colors }
    }
}

impl SequenceRef {
    /// The reference playing `count` passes; `None` for a count the
    /// player refuses (none, or not a whole number), by its own rule
    /// ([`parse_sequence_loop_param`]).
    pub fn with_loop_count(&self, count: f64) -> Option<SequenceRef> {
        let written = number(count);
        match parse_sequence_loop_param(&written) {
            Ok(SequenceLoop::Count(_)) => Some(SequenceRef {
                loop_: Some(written),
                ..self.clone()
            }),
            _ => None,
        }
    }
}

/// The effect a click on the timeline adds: a `static` on every group,
/// a measure long on a show with a tempo map (`by_bars`), else five
/// seconds, on `layer` when the lane is one.
pub fn new_effect(layer: Option<&str>, by_bars: bool) -> CueEffect {
    let param = |name: &str, value: &str| Param {
        name: name.to_string(),
        value: value.to_string(),
        is_ref: None,
        remarks: Remarks::default(),
    };
    let duration = if by_bars {
        length_text(1.0, DurationUnit::Measures)
    } else {
        length_text(5.0, DurationUnit::S)
    };
    let mut params = vec![param("duration", &duration)];
    if let Some(layer) = layer.filter(|l| !l.is_empty()) {
        params.push(param("layer", layer));
    }
    CueEffect {
        groups: vec!["all".to_string()],
        group_remarks: None,
        effect: EffectParams {
            kind: EffectKind::Static,
            params,
            no_comma: None,
        },
        comments_before: None,
        comment: None,
        comments_after: None,
    }
}

/// The name the first `fixture_type` line of `source` declares, as the
/// grammar reads it (`fixture_type_name`); `None` when no line declares
/// one.
pub fn declared_fixture_type(source: &str) -> Option<String> {
    let rest = source.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("fixture_type")?;
        rest.starts_with([' ', '\t']).then_some(rest.trim_start())
    })?;
    let name = LightingParser::parse(Rule::fixture_type_name, rest)
        .ok()?
        .next()?;
    Some(unquoted(name.as_str()).to_string())
}

/// Whether `text`, whole, is a sequence parameter reference (`$name`) by
/// the grammar's `param_ref`.
pub fn param_ref(text: &str) -> bool {
    LightingParser::parse(Rule::param_ref_only, text.trim()).is_ok()
}

/// A string value in quotes, unless it is a parameter, which must stay
/// bare.
pub fn quoted(value: &str) -> String {
    if param_ref(value) {
        value.trim().to_string()
    } else {
        format!("\"{value}\"")
    }
}

/// An angle as the DSL wants it: as typed when the grammar reads it as
/// one (`45deg`, a `$param`), else a bare number with the unit added.
pub fn with_deg(value: &str) -> String {
    let v = value.trim();
    let degrees = LightingParser::parse(Rule::degrees_parameter, v)
        .ok()
        .and_then(|mut pairs| pairs.next())
        .is_some_and(|pair| pair.as_span().end() == v.len());
    if degrees || param_ref(v) {
        v.to_string()
    } else {
        format!("{v}deg")
    }
}

#[cfg(test)]
mod tests {
    use super::super::document::document;
    use super::*;

    fn effect(line: &str) -> CueEffect {
        let source = format!("show \"S\" {{\n    @1/1\n    {line}\n}}\n");
        document(&source, None).document.unwrap().shows[0].cues[0].effects[0].clone()
    }

    fn values(e: &CueEffect) -> Vec<(String, String)> {
        e.effect
            .params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn a_move_aims_at_a_focus_point_or_at_angles_never_both() {
        let e = effect("m: move, pan: 10deg, tilt: 5deg, duration: 1s");
        let focused = e.edited(&EffectEdit::Move {
            key: MoveParam::Focus,
            value: Some("stage".to_string()),
        });
        assert_eq!(
            values(&focused),
            pairs(&[("duration", "1s"), ("focus", "\"stage\"")])
        );
        let panned = focused.edited(&EffectEdit::Move {
            key: MoveParam::Pan,
            value: Some(" 45 ".to_string()),
        });
        assert_eq!(
            values(&panned),
            pairs(&[("duration", "1s"), ("pan", "45deg")])
        );
        let param = panned.edited(&EffectEdit::Move {
            key: MoveParam::FromTilt,
            value: Some("$t".to_string()),
        });
        assert_eq!(
            values(&param),
            pairs(&[("duration", "1s"), ("pan", "45deg"), ("from_tilt", "$t")])
        );
        let started = param.edited(&EffectEdit::Move {
            key: MoveParam::From,
            value: Some("$spot".to_string()),
        });
        assert_eq!(
            values(&started),
            pairs(&[("duration", "1s"), ("pan", "45deg"), ("from", "$spot")])
        );
    }

    #[test]
    fn colours_replace_the_colours_where_the_first_was() {
        let e = effect("a: cycle, speed: 1, color: \"red\", color: \"blue\", duration: 1s");
        let c = e.edited(&EffectEdit::Colors {
            colors: vec!["green".to_string(), "$c".to_string(), "#FF0000".to_string()],
        });
        assert_eq!(
            values(&c),
            pairs(&[
                ("speed", "1"),
                ("color", "\"green\""),
                ("color", "$c"),
                ("color", "\"#FF0000\""),
                ("duration", "1s"),
            ])
        );
        assert_eq!(c.effect.view().colors, vec!["green", "$c", "#FF0000"]);
    }

    #[test]
    fn a_form_shows_the_last_value_of_each_name_unquoted_where_quoted() {
        let view = effect("m: move, to: \"a\", dimmer: 10%, dimmer: 20%, duration: 1s")
            .effect
            .view();
        assert_eq!(view.values["focus"], "a");
        assert_eq!(view.values["from"], "");
        assert_eq!(view.values["dimmer"], "20%");
        assert_eq!(unquoted("\"x"), "x");
        assert_eq!(unquoted("\"\""), "");
    }

    #[test]
    fn a_loop_count_the_player_refuses_is_never_written() {
        let r = document("show \"S\" {\n    @1/1\n    sequence \"x\"\n}\n", None)
            .document
            .unwrap()
            .shows[0]
            .cues[0]
            .sequences[0]
            .clone();
        assert_eq!(r.with_loop_count(3.0).unwrap().loop_.as_deref(), Some("3"));
        for count in [0.0, -1.0, 1.5, f64::NAN, f64::INFINITY, 1e30] {
            assert!(r.with_loop_count(count).is_none(), "{count}");
        }
        // As many passes as the player takes, on every target: past `u32`.
        assert_eq!(
            r.with_loop_count(5e9).unwrap().loop_.as_deref(),
            Some("5000000000")
        );
    }

    #[test]
    fn a_new_effect_lasts_a_bar_with_a_tempo_and_five_seconds_without() {
        let e = new_effect(Some("foreground"), true);
        assert_eq!(
            values(&e),
            pairs(&[("duration", "1measure"), ("layer", "foreground")])
        );
        assert_eq!(
            values(&new_effect(None, false)),
            pairs(&[("duration", "5s")])
        );
    }
}
