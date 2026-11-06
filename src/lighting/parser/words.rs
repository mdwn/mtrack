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
//! The parser's words, for the text that restates them: the MCP DSL
//! reference is written from these, and the docs are tested against them.
//! (The player's alone: the web UI's rules module reads
//! [`super::edit::vocabulary`].)

use super::super::effects::{EffectType, TempoAwareSpeed};
use super::effect_parse::effect_with_defaults;
use super::types::EffectKind;
use super::writer::number;

use super::effect_parse::{
    CHASE_DIRECTIONS, CHASE_PATTERNS, CYCLE_DIRECTIONS, DIMMER_CURVES, EASINGS,
};
pub(crate) use super::effect_parse::{
    CHASE_TRANSITIONS, COMMON_PARAMS, CYCLE_TRANSITIONS, PER_KEYWORDS,
};
pub(crate) use super::types::LayerCommandParam;
pub(crate) use super::utils::UNITS;

/// The keyword `table` writes `value` as (the first, where a value has
/// more than one).
fn keyword_of<T>(table: &[(T, &'static str)], value: &T) -> &'static str {
    table
        .iter()
        .find(|(t, _)| std::mem::discriminant(t) == std::mem::discriminant(value))
        .map(|(_, k)| *k)
        .expect("every value has a keyword")
}

/// Each parameter an effect gives a value when its line does not, with
/// that value as the DSL writes it: what [`effect_with_defaults`] sets, in
/// the order [`EffectKind::params`] lists them. A parameter absent here
/// has no default (`duration`, but on a `dimmer`, must be written).
pub(crate) fn effect_defaults(kind: EffectKind) -> Vec<(&'static str, String)> {
    let rate = |value: &TempoAwareSpeed| match value {
        TempoAwareSpeed::Fixed(n) => number(*n),
        TempoAwareSpeed::Seconds(n) => format!("{}s", number(*n)),
        TempoAwareSpeed::Beats(n) => format!("{}beats", number(*n)),
        TempoAwareSpeed::Measures(n) => format!("{}measures", number(*n)),
    };
    let level = |n: f64| format!("{}%", number(n * 100.0));
    let mut out: Vec<(&'static str, String)> = Vec::new();
    match effect_with_defaults(kind.keyword()).expect("every effect has its defaults") {
        EffectType::Static { .. } => {}
        EffectType::ColorCycle {
            speed,
            direction,
            transition,
            ..
        } => {
            out.push(("speed", rate(&speed)));
            out.push((
                "direction",
                keyword_of(&CYCLE_DIRECTIONS, &direction).into(),
            ));
            out.push((
                "transition",
                keyword_of(&CYCLE_TRANSITIONS, &transition).into(),
            ));
        }
        EffectType::Strobe { frequency, .. } => out.push(("frequency", rate(&frequency))),
        EffectType::Pulse {
            base_level,
            pulse_amplitude,
            frequency,
            ..
        } => {
            out.push(("base_level", level(base_level)));
            out.push(("pulse_amplitude", level(pulse_amplitude)));
            out.push(("frequency", rate(&frequency)));
        }
        EffectType::Chase {
            pattern,
            speed,
            direction,
            transition,
            ..
        } => {
            out.push(("pattern", keyword_of(&CHASE_PATTERNS, &pattern).into()));
            out.push(("speed", rate(&speed)));
            out.push((
                "direction",
                keyword_of(&CHASE_DIRECTIONS, &direction).into(),
            ));
            out.push((
                "transition",
                keyword_of(&CHASE_TRANSITIONS, &transition).into(),
            ));
        }
        EffectType::Dimmer {
            start_level,
            end_level,
            duration,
            curve,
        } => {
            out.push(("start_level", level(start_level)));
            out.push(("end_level", level(end_level)));
            out.push(("curve", keyword_of(&DIMMER_CURVES, &curve).into()));
            out.push(("duration", format!("{}s", number(duration.as_secs_f64()))));
        }
        EffectType::Rainbow {
            speed,
            saturation,
            brightness,
            ..
        } => {
            out.push(("speed", rate(&speed)));
            out.push(("saturation", level(saturation)));
            out.push(("brightness", level(brightness)));
        }
        EffectType::Move { easing, .. } => {
            out.push(("easing", keyword_of(&EASINGS, &easing).into()))
        }
    }
    let order = |name: &str| kind.params().iter().position(|p| *p == name);
    out.sort_by_key(|(name, _)| order(name));
    out
}
