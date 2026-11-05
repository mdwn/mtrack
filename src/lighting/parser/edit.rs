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
use super::document::{CueEffect, EffectKind, EffectParams, Param, Remarks, SequenceRef};
use super::grammar::{LightingParser, Rule};
use super::utils::unquoted;
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
    /// A `move`'s target: a focus point or an angle, setting one clearing
    /// the other.
    MoveTarget {
        key: MoveTarget,
        #[serde(default)]
        #[cfg_attr(feature = "tsify", tsify(optional))]
        value: Option<String>,
    },
    /// A `move`'s start: a focus point or angles, setting one clearing the
    /// other.
    MoveStart {
        key: MoveStart,
        #[serde(default)]
        #[cfg_attr(feature = "tsify", tsify(optional))]
        value: Option<String>,
    },
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MoveTarget {
    Focus,
    Pan,
    Tilt,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MoveStart {
    From,
    FromPan,
    FromTilt,
}

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
            EffectEdit::MoveTarget { key, value } => {
                let value = value.as_deref().filter(|v| !v.is_empty());
                let mut next = self.clone();
                match key {
                    MoveTarget::Focus => {
                        if value.is_some() {
                            next = next.without_params(&["pan", "tilt", "to"]);
                        }
                        next.with_param("focus", value.map(quoted).as_deref())
                    }
                    MoveTarget::Pan | MoveTarget::Tilt => {
                        if value.is_some() {
                            next = next.without_params(&["focus", "to"]);
                        }
                        let name = if *key == MoveTarget::Pan {
                            "pan"
                        } else {
                            "tilt"
                        };
                        next.with_param(name, value.map(with_deg).as_deref())
                    }
                }
            }
            EffectEdit::MoveStart { key, value } => {
                let value = value.as_deref().filter(|v| !v.is_empty());
                let mut next = self.clone();
                match key {
                    MoveStart::From => {
                        if value.is_some() {
                            next = next.without_params(&["from_pan", "from_tilt"]);
                        }
                        next.with_param("from", value.map(quoted).as_deref())
                    }
                    MoveStart::FromPan | MoveStart::FromTilt => {
                        if value.is_some() {
                            next = next.without_params(&["from"]);
                        }
                        let name = if *key == MoveStart::FromPan {
                            "from_pan"
                        } else {
                            "from_tilt"
                        };
                        next.with_param(name, value.map(with_deg).as_deref())
                    }
                }
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
        let (kept, orphans) = dropped(params, drop);
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
        if !values.contains_key("focus") {
            if let Some(to) = values.get("to").cloned() {
                values.insert("focus".to_string(), to);
            }
        }
        for name in ["focus", "from"] {
            let shown = values.get(name).map(|v| unquoted(v).to_string());
            values.insert(name.to_string(), shown.unwrap_or_default());
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
    /// player refuses (none, or not a whole number).
    pub fn with_loop_count(&self, count: f64) -> Option<SequenceRef> {
        if !(count >= 1.0 && count.fract() == 0.0 && count <= f64::from(u32::MAX)) {
            return None;
        }
        Some(SequenceRef {
            loop_: Some(number(count)),
            ..self.clone()
        })
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
        let focused = e.edited(&EffectEdit::MoveTarget {
            key: MoveTarget::Focus,
            value: Some("stage".to_string()),
        });
        assert_eq!(
            values(&focused),
            pairs(&[("duration", "1s"), ("focus", "\"stage\"")])
        );
        let panned = focused.edited(&EffectEdit::MoveTarget {
            key: MoveTarget::Pan,
            value: Some(" 45 ".to_string()),
        });
        assert_eq!(
            values(&panned),
            pairs(&[("duration", "1s"), ("pan", "45deg")])
        );
        let param = panned.edited(&EffectEdit::MoveStart {
            key: MoveStart::FromTilt,
            value: Some("$t".to_string()),
        });
        assert_eq!(
            values(&param),
            pairs(&[("duration", "1s"), ("pan", "45deg"), ("from_tilt", "$t")])
        );
        let started = param.edited(&EffectEdit::MoveStart {
            key: MoveStart::From,
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
        for count in [0.0, -1.0, 1.5, f64::NAN, 1e10] {
            assert!(r.with_loop_count(count).is_none(), "{count}");
        }
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
