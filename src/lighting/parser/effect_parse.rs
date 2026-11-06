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

use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use super::super::effects::{
    ChaseDirection, ChasePattern, Color, CycleDirection, CycleTransition, DimmerCurve, Easing,
    EffectType, MoveTarget, TempoAwareFrequency, TempoAwareSpeed,
};
use super::grammar::Rule;
use super::types::{Effect, EffectKind, ParseContext};
use super::utils::{
    parse_blend_mode, parse_color_string, parse_frequency_string, parse_layer,
    parse_percentage_to_f64, parse_speed_string, split_unit, unquoted,
};
use pest::iterators::Pair;

/// Helper to convert color to normalized RGB parameters
fn color_to_normalized_params(color: &Color) -> (f64, f64, f64) {
    (
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
    )
}

/// A `color:` value, or an error naming it.
///
/// An unparseable colour used to be dropped without a word: a `static` set no
/// colour at all and rendered as if it were not there, and a `cycle` ran on
/// its other colours -- while the show validated clean (#538). It is an error
/// like any other bad parameter value.
fn parse_color_value(value: &str) -> Result<crate::lighting::effects::Color, Box<dyn Error>> {
    parse_color_string(value).ok_or_else(|| {
        format!(
            "Invalid color '{}' (expected #RRGGBB, rgb(r, g, b) or a color name)",
            value.trim_matches('"')
        )
        .into()
    })
}

/// A level parameter's value -- a number or a percentage -- or an error
/// naming it. A value that did not parse used to be dropped, leaving the
/// effect at its default level with no word about why (the same silence as
/// an invalid colour, #538).
fn parse_level(key: &str, value: &str) -> Result<f64, Box<dyn Error>> {
    parse_percentage_to_f64(value).map_err(|_| {
        format!(
            "Invalid {key} '{}' (expected a number or a percentage, e.g. 0.5 or 50%)",
            value.trim_matches('"')
        )
        .into()
    })
}

/// Helper to clean and normalize string values (strip quotes, trim, lowercase)
fn clean_string_value(value: &str) -> String {
    value.trim_matches('"').trim().to_lowercase()
}

/// Each cycle direction and its keyword, as the grammar writes it.
pub(crate) const CYCLE_DIRECTIONS: [(CycleDirection, &str); 3] = [
    (CycleDirection::Forward, "forward"),
    (CycleDirection::Backward, "backward"),
    (CycleDirection::PingPong, "pingpong"),
];

/// Each chase pattern and its keyword.
pub(crate) const CHASE_PATTERNS: [(ChasePattern, &str); 3] = [
    (ChasePattern::Linear, "linear"),
    (ChasePattern::Snake, "snake"),
    (ChasePattern::Random, "random"),
];

/// Each chase direction and its keyword.
pub(crate) const CHASE_DIRECTIONS: [(ChaseDirection, &str); 6] = [
    (ChaseDirection::LeftToRight, "left_to_right"),
    (ChaseDirection::RightToLeft, "right_to_left"),
    (ChaseDirection::TopToBottom, "top_to_bottom"),
    (ChaseDirection::BottomToTop, "bottom_to_top"),
    (ChaseDirection::Clockwise, "clockwise"),
    (ChaseDirection::CounterClockwise, "counter_clockwise"),
];

/// Each dimmer curve and its keyword.
pub(crate) const DIMMER_CURVES: [(DimmerCurve, &str); 5] = [
    (DimmerCurve::Linear, "linear"),
    (DimmerCurve::Exponential, "exponential"),
    (DimmerCurve::Logarithmic, "logarithmic"),
    (DimmerCurve::Sine, "sine"),
    (DimmerCurve::Cosine, "cosine"),
];

/// Each move easing and its keyword.
pub(crate) const EASINGS: [(Easing, &str); 2] =
    [(Easing::Linear, "linear"), (Easing::Smooth, "smooth")];

/// The parameters each effect reads, beyond the ones every effect takes
/// ([`COMMON_PARAMS`]): the rule `apply_parameters_to_effect_type` reads
/// (any other is reported unused, or on a `static` read as a channel
/// level), and what the editor offers and keeps when an effect changes
/// kind. A cycle's colours are read with the common parameters.
const EFFECT_PARAMS: [(EffectKind, &[&str]); 8] = [
    (
        EffectKind::Static,
        &[
            "dimmer",
            "intensity",
            "red",
            "green",
            "blue",
            "white",
            "color",
            "duration",
        ],
    ),
    (
        EffectKind::Cycle,
        &["speed", "direction", "transition", "color", "duration"],
    ),
    (EffectKind::Strobe, &["frequency", "rate", "duration"]),
    (
        EffectKind::Pulse,
        &[
            "base_level",
            "pulse_amplitude",
            "intensity",
            "frequency",
            "duration",
        ],
    ),
    (
        EffectKind::Chase,
        &["pattern", "speed", "direction", "transition", "duration"],
    ),
    (
        EffectKind::Dimmer,
        &[
            "start",
            "start_level",
            "end",
            "end_level",
            "curve",
            "duration",
        ],
    ),
    (
        EffectKind::Rainbow,
        &["speed", "saturation", "brightness", "duration"],
    ),
    (
        EffectKind::Move,
        &[
            "focus",
            "to",
            "from",
            "from_pan",
            "from_tilt",
            "pan",
            "tilt",
            "easing",
            "duration",
        ],
    ),
];

/// A `move`'s target and start parameters.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(serde::Deserialize, serde::Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MoveParam {
    Focus,
    Pan,
    Tilt,
    From,
    FromPan,
    FromTilt,
}

/// The end of a move a parameter sets: where it goes, or where it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MoveEnd {
    To,
    From,
}

/// What of its end a parameter sets: a quoted focus point, or an angle.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MoveSlot {
    Point,
    Pan,
    Tilt,
}

/// Each `move` parameter: its name, the end it sets and what of it. An
/// end is a point or angles, never both. The one table: the parser reads
/// a move by it, and the editor clears and quotes by it.
const MOVE_PARAMS: [(MoveParam, &str, MoveEnd, MoveSlot); 6] = [
    (MoveParam::Focus, "focus", MoveEnd::To, MoveSlot::Point),
    (MoveParam::Pan, "pan", MoveEnd::To, MoveSlot::Pan),
    (MoveParam::Tilt, "tilt", MoveEnd::To, MoveSlot::Tilt),
    (MoveParam::From, "from", MoveEnd::From, MoveSlot::Point),
    (MoveParam::FromPan, "from_pan", MoveEnd::From, MoveSlot::Pan),
    (
        MoveParam::FromTilt,
        "from_tilt",
        MoveEnd::From,
        MoveSlot::Tilt,
    ),
];

/// Other names a `move` parameter is written under: `to` for `focus`.
pub(crate) const MOVE_ALIASES: [(&str, MoveParam); 1] = [("to", MoveParam::Focus)];

impl MoveParam {
    fn row(self) -> (MoveParam, &'static str, MoveEnd, MoveSlot) {
        *MOVE_PARAMS
            .iter()
            .find(|(param, ..)| *param == self)
            .expect("every move parameter")
    }

    /// The parameter `name` (or an alias of it) is.
    fn named(name: &str) -> Option<MoveParam> {
        MOVE_PARAMS
            .iter()
            .find(|(_, n, ..)| *n == name)
            .map(|(param, ..)| *param)
            .or_else(|| {
                MOVE_ALIASES
                    .iter()
                    .find(|(alias, _)| *alias == name)
                    .map(|(_, param)| *param)
            })
    }

    /// The parameter of `end` that sets `slot`.
    fn of(end: MoveEnd, slot: MoveSlot) -> MoveParam {
        MOVE_PARAMS
            .iter()
            .find(|(_, _, e, s)| *e == end && *s == slot)
            .map(|(param, ..)| *param)
            .expect("every end has every slot")
    }

    /// The name it is written under.
    pub(crate) fn name(self) -> &'static str {
        self.row().1
    }

    /// The parameters that are a focus point.
    pub(crate) fn points() -> impl Iterator<Item = MoveParam> {
        MOVE_PARAMS
            .iter()
            .filter(|(.., slot)| *slot == MoveSlot::Point)
            .map(|(param, ..)| *param)
    }

    /// Whether its value is a quoted focus point (else an angle).
    pub(crate) fn is_point(self) -> bool {
        self.row().3 == MoveSlot::Point
    }

    /// Every name setting it gives no longer applies: its aliases, and on
    /// its end the angles a point excludes, or the point angles exclude.
    pub(crate) fn excludes(self) -> Vec<&'static str> {
        let (_, _, end, _) = self.row();
        let aliases = MOVE_ALIASES
            .iter()
            .filter(|(_, param)| *param == self)
            .map(|(alias, _)| *alias);
        let others = MOVE_PARAMS
            .iter()
            .filter(|(param, _, e, _)| *e == end && param.is_point() != self.is_point())
            .flat_map(|(param, name, ..)| {
                std::iter::once(*name).chain(
                    MOVE_ALIASES
                        .iter()
                        .filter(move |(_, p)| p == param)
                        .map(|(alias, _)| *alias),
                )
            });
        aliases.chain(others).collect()
    }
}

/// One end of a move as written: its point, pan and tilt.
#[derive(Default)]
struct MoveEndWritten {
    point: Option<String>,
    pan: Option<f64>,
    tilt: Option<f64>,
}

impl MoveEndWritten {
    /// The end, `None` for none written; an error naming its parameters
    /// for both a point and angles.
    fn target(self, end: MoveEnd) -> Result<Option<MoveTarget>, String> {
        let name = |slot| MoveParam::of(end, slot).name();
        match (self.point, self.pan, self.tilt) {
            (Some(_), pan, tilt) if pan.is_some() || tilt.is_some() => Err(format!(
                "move: give either `{}` or `{}`/`{}`, not both",
                name(MoveSlot::Point),
                name(MoveSlot::Pan),
                name(MoveSlot::Tilt)
            )),
            (Some(point), _, _) => Ok(Some(MoveTarget::Focus(point))),
            (None, None, None) => Ok(None),
            (None, pan, tilt) => Ok(Some(MoveTarget::Angles { pan, tilt })),
        }
    }
}

/// The parameters `parse_effect_definition` reads for every effect
/// (`spread` does something on a cycle or a rainbow alone, which lint says).
pub(crate) const COMMON_PARAMS: [&str; 7] = [
    "layer",
    "blend_mode",
    "up_time",
    "hold_time",
    "down_time",
    "per",
    "spread",
];

impl EffectKind {
    /// The parameters the effect reads, beyond [`COMMON_PARAMS`].
    pub fn params(self) -> &'static [&'static str] {
        EFFECT_PARAMS
            .iter()
            .find(|(kind, _)| *kind == self)
            .map(|(_, params)| *params)
            .expect("every effect has its parameters")
    }
}

/// The value `table` names `keyword` by, or an error: `lead`, the word,
/// and what it may be.
fn keyword_value<T: Clone>(
    table: &[(T, &str)],
    keyword: &str,
    lead: &str,
) -> Result<T, Box<dyn Error>> {
    match table.iter().find(|(_, k)| *k == keyword) {
        Some((value, _)) => Ok(value.clone()),
        None => {
            let expected: Vec<&str> = table.iter().map(|(_, k)| *k).collect();
            Err(format!("{lead} '{keyword}' (expected: {})", expected.join(", ")).into())
        }
    }
}

/// Parses one effect line. An error names the line it is on, so a bad value
/// in a long show can be found (#538).
pub(crate) fn parse_effect_definition(
    pair: Pair<Rule>,
    ctx: &ParseContext,
) -> Result<Effect, Box<dyn Error>> {
    let (line, _) = pair.line_col();
    parse_effect_line(pair, ctx).map_err(|e| format!("line {line}: {e}").into())
}

fn parse_effect_line(pair: Pair<Rule>, ctx: &ParseContext) -> Result<Effect, Box<dyn Error>> {
    let mut groups = Vec::new();
    let mut effect_type = EffectType::Static {
        parameters: HashMap::new(),
        duration: Duration::ZERO,
    };
    let mut parameters = HashMap::new();
    let mut color_parameters = Vec::new();
    let mut layer = None;
    let mut blend_mode = None;
    let mut up_time = None;
    let mut hold_time = None;
    let mut down_time = None;
    let mut musical = super::types::MusicalDurations::default();
    let mut per_cell = false;
    let mut spread = 0.0_f64;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::group_list => {
                for group_pair in inner_pair.into_inner() {
                    if group_pair.as_rule() == Rule::group_name {
                        groups.push(group_pair.as_str().trim().to_string());
                    }
                }
            }
            Rule::effect_type => {
                effect_type = effect_with_defaults(inner_pair.as_str())
                    .ok_or_else(|| format!("Unknown effect type: {}", inner_pair.as_str()))?;
            }
            Rule::parameters => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::parameter {
                        let (key, value) = super::utils::parse_parameter(param_pair)?;
                        musical.record(&key, &value);
                        match key.as_str() {
                            "color" if matches!(effect_type, EffectType::ColorCycle { .. }) => {
                                color_parameters.push(value);
                            }
                            "layer" => {
                                layer = Some(parse_layer(&value).ok_or_else(|| {
                                    format!("Invalid layer: '{}' (expected: background, midground, foreground)", value)
                                })?);
                            }
                            "blend_mode" => {
                                blend_mode = Some(
                                    parse_blend_mode(&value)
                                        .ok_or_else(|| format!("Invalid blend mode: {}", value))?,
                                );
                            }
                            "up_time" => up_time = Some(ctx.duration(&value)?),
                            "hold_time" => hold_time = Some(ctx.duration(&value)?),
                            "per" => {
                                per_cell = match value.as_str() {
                                    "cell" | "cells" => true,
                                    "fixture" | "fixtures" => false,
                                    other => {
                                        return Err(format!(
                                            "Invalid per: '{other}' (expected: fixture or cell)"
                                        )
                                        .into())
                                    }
                                };
                            }
                            "spread" => {
                                spread = parse_degrees(&value)?;
                            }
                            "down_time" => down_time = Some(ctx.duration(&value)?),
                            _ => {
                                parameters.insert(key, value);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Apply parameters to the effect type
    let (final_effect_type, ignored_parameters) =
        apply_parameters_to_effect_type(effect_type, &parameters, &color_parameters, ctx)?;

    // A move's duration is its travel time, which `hold_time` cannot stand
    // in for: without one the move would be an instant snap held for the
    // hold, which is not what "required" means.
    if let EffectType::Move { duration, .. } = &final_effect_type {
        if duration.is_zero() {
            return Err(
                "Effect 'move' requires a 'duration' parameter: the travel time \
                        (hold_time does not stand in for it)."
                    .into(),
            );
        }
    }

    // Validate that every effect has an explicit duration.
    // Dimmer always has a duration (defaults to 1s). For all other types,
    // either the effect's duration field or hold_time must be set.
    if !matches!(&final_effect_type, EffectType::Dimmer { .. }) {
        let effect_duration = final_effect_type.duration();
        if effect_duration.is_zero() && hold_time.is_none() {
            return Err(format!(
                "Effect '{}' requires a 'duration' or 'hold_time' parameter. \
                 All effects must have an explicit, finite duration.",
                effect_keyword(&final_effect_type)
            )
            .into());
        }
    }

    Ok(Effect {
        groups,
        effect_type: final_effect_type,
        layer,
        blend_mode,
        up_time,
        hold_time,
        down_time,
        sequences: Vec::new(), // Set as the sequences it is in are expanded
        per_cell,
        spread,
        ignored_parameters,
        tempo_offset_secs: ctx.offset_secs,
        musical,
    })
}

/// The effect an author's keyword names, with every parameter at its
/// default, or `None` for a word that names none.
fn effect_with_defaults(keyword: &str) -> Option<EffectType> {
    Some(match EffectKind::from_keyword(keyword)? {
        EffectKind::Static => EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        },
        EffectKind::Cycle => EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        },
        EffectKind::Strobe => EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        },
        EffectKind::Pulse => EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        },
        EffectKind::Chase => EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        },
        EffectKind::Dimmer => EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        EffectKind::Rainbow => EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::ZERO,
        },
        EffectKind::Move => EffectType::Move {
            to: MoveTarget::Angles {
                pan: None,
                tilt: None,
            },
            from: None,
            easing: Easing::default(),
            duration: Duration::ZERO,
        },
    })
}

/// The keyword an author writes for an effect type: what
/// [`effect_with_defaults`] reads. (`EffectType::name` is the type's own
/// spelling, `ColorCycle` for `cycle`.)
pub(crate) fn effect_keyword(effect_type: &EffectType) -> &'static str {
    EffectKind::of(effect_type).keyword()
}

/// Parses an angle written with its unit: `45deg`, `-20.5deg`.
fn parse_degrees(value: &str) -> Result<f64, Box<dyn Error>> {
    let (number, unit) = split_unit(value);
    if unit != "deg" {
        return Err(format!("angle '{value}' must be written in degrees, e.g. 45deg").into());
    }
    number
        .trim()
        .parse::<f64>()
        .map_err(|e| format!("invalid angle '{value}': {e}").into())
}

/// Applies parsed parameters to effect types
/// Applies the effect-specific parameters, and reports the ones the effect type
/// does not use.
///
/// The common parameters (`layer`, `blend_mode`, the fade times, and `color` on
/// a cycle) are consumed by the caller before this runs, so anything left
/// unmatched here is a parameter that names nothing on this effect — like a
/// `direction` on a rainbow, which parsed happily and did nothing. Returned
/// rather than dropped so a lint can say so; rejecting outright would break
/// shows that already carry one.
pub(crate) fn apply_parameters_to_effect_type(
    mut effect_type: EffectType,
    parameters: &HashMap<String, String>,
    color_parameters: &[String],
    ctx: &ParseContext,
) -> Result<(EffectType, Vec<String>), Box<dyn Error>> {
    let mut ignored: Vec<String> = Vec::new();
    // The effect reads the parameters its table names; a `static` also
    // takes any other as a channel level, which its arm decides.
    let kind = EffectKind::of(&effect_type);
    let parameters: HashMap<&String, &String> = parameters
        .iter()
        .filter(|(key, _)| {
            let read = kind == EffectKind::Static || kind.params().contains(&key.as_str());
            if !read {
                ignored.push(key.to_string());
            }
            read
        })
        .collect();
    // A name the table gives an effect whose arm does not read it.
    let unread = |name: &str| format!("`{name}` is in {kind:?}'s parameters but is not read");
    let tempo_map = &ctx.tempo_map;
    match &mut effect_type {
        EffectType::Static {
            parameters: static_params,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    // `intensity` is the documented common parameter, `dimmer` the
                    // static-specific spelling. They mean the same level.
                    "dimmer" | "intensity" => {
                        let val = parse_level(key, value)?;
                        static_params.insert("dimmer".to_string(), val);
                    }
                    "red" | "green" | "blue" | "white" => {
                        let val = parse_level(key, value)?;
                        static_params.insert(key.clone(), val);
                    }
                    "color" => {
                        let color = parse_color_value(value)?;
                        let (r, g, b) = color_to_normalized_params(&color);
                        static_params.insert("red".to_string(), r);
                        static_params.insert("green".to_string(), g);
                        static_params.insert("blue".to_string(), b);
                    }
                    "duration" => *duration = ctx.duration(value)?,
                    other => {
                        // A bare number is a channel level, which `static`
                        // accepts by name — that is how a fixture's own channels
                        // are addressed. Anything else is not used by anything,
                        // and `static` is where a typo is most likely, so it
                        // must not be the one effect type the lint cannot see.
                        // A percentage is as valid a channel level as a bare
                        // float — `gobo: 50%` was dropped silently, and the
                        // warning then blamed the name when the fault was the
                        // `%`.
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            static_params.insert(key.clone(), val);
                        } else {
                            ignored.push(other.to_string());
                        }
                    }
                }
            }
        }
        EffectType::ColorCycle {
            colors,
            speed,
            direction,
            transition,
            duration,
        } => {
            // Add all color parameters
            for color_str in color_parameters {
                colors.push(parse_color_value(color_str)?);
            }

            // Handle other parameters
            for (key, value) in parameters {
                match key.as_str() {
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "direction" => {
                        *direction = keyword_value(
                            &CYCLE_DIRECTIONS,
                            &clean_string_value(value),
                            "Invalid cycle direction:",
                        )?;
                    }
                    "transition" => {
                        *transition = match clean_string_value(value).as_str() {
                            "snap" => CycleTransition::Snap,
                            "fade" | "crossfade" => CycleTransition::Fade,
                            "hue" => CycleTransition::Hue,
                            other => {
                                return Err(format!(
                                "Invalid transition: '{}' (expected: snap, fade, crossfade, hue)",
                                other
                            )
                                .into())
                            }
                        };
                    }
                    "duration" => *duration = ctx.duration(value)?,
                    other => return Err(unread(other).into()),
                }
            }
        }
        EffectType::Strobe {
            frequency,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "frequency" | "rate" => match parse_frequency_string(value, tempo_map) {
                        Ok(val) => *frequency = val,
                        Err(e) => {
                            return Err(
                                format!("Invalid frequency value '{}': {}", value, e).into()
                            );
                        }
                    },
                    "duration" => *duration = ctx.duration(value)?,
                    other => return Err(unread(other).into()),
                }
            }
        }
        EffectType::Pulse {
            base_level,
            pulse_amplitude,
            frequency,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "base_level" => {
                        let val = parse_level(key, value)?;
                        *base_level = val;
                    }
                    "pulse_amplitude" | "intensity" => {
                        let val = parse_level(key, value)?;
                        *pulse_amplitude = val;
                    }
                    "frequency" => match parse_frequency_string(value, tempo_map) {
                        Ok(val) => *frequency = val,
                        Err(e) => {
                            return Err(
                                format!("Invalid frequency value '{}': {}", value, e).into()
                            );
                        }
                    },
                    "duration" => *duration = ctx.duration(value)?,
                    other => return Err(unread(other).into()),
                }
            }
        }
        EffectType::Chase {
            pattern,
            speed,
            direction,
            transition,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "pattern" => {
                        *pattern = keyword_value(
                            &CHASE_PATTERNS,
                            &clean_string_value(value),
                            "Invalid chase pattern:",
                        )?;
                    }
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "direction" => {
                        *direction = keyword_value(
                            &CHASE_DIRECTIONS,
                            &clean_string_value(value),
                            "Invalid chase direction:",
                        )?;
                    }
                    "transition" => {
                        *transition = match clean_string_value(value).as_str() {
                            "snap" => CycleTransition::Snap,
                            "fade" | "crossfade" => CycleTransition::Fade,
                            other => {
                                return Err(format!(
                                    "Invalid transition: '{}' (expected: snap, fade, crossfade)",
                                    other
                                )
                                .into())
                            }
                        };
                    }
                    "duration" => *duration = ctx.duration(value)?,
                    other => return Err(unread(other).into()),
                }
            }
        }
        EffectType::Dimmer {
            start_level,
            end_level,
            duration,
            curve,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "start" | "start_level" => {
                        let val = parse_level(key, value)?;
                        *start_level = val;
                    }
                    "end" | "end_level" => {
                        let val = parse_level(key, value)?;
                        *end_level = val;
                    }
                    "duration" => *duration = ctx.duration(value)?,
                    "curve" => {
                        *curve = keyword_value(&DIMMER_CURVES, value, "Invalid dimmer curve:")?
                    }
                    other => return Err(unread(other).into()),
                }
            }
        }
        EffectType::Move {
            to,
            from,
            easing,
            duration,
        } => {
            let mut ends = [MoveEndWritten::default(), MoveEndWritten::default()];
            for (key, value) in parameters {
                if let Some(param) = MoveParam::named(key) {
                    let (_, _, end, slot) = param.row();
                    let written = &mut ends[end as usize];
                    match slot {
                        MoveSlot::Point => written.point = Some(unquoted(value).to_string()),
                        MoveSlot::Pan => written.pan = Some(parse_degrees(value)?),
                        MoveSlot::Tilt => written.tilt = Some(parse_degrees(value)?),
                    }
                    continue;
                }
                match key.as_str() {
                    "easing" => *easing = keyword_value(&EASINGS, value, "Invalid easing")?,
                    "duration" => *duration = ctx.duration(value)?,
                    other => return Err(unread(other).into()),
                }
            }
            let [to_written, from_written] = ends;
            *to = to_written.target(MoveEnd::To)?.ok_or_else(|| {
                let name = |slot| MoveParam::of(MoveEnd::To, slot).name();
                let aliases: Vec<&str> = MOVE_ALIASES.iter().map(|(alias, _)| *alias).collect();
                format!(
                    "move: requires a `{}` (or `{}`), or `{}`/`{}`",
                    name(MoveSlot::Point),
                    aliases.join("`, `"),
                    name(MoveSlot::Pan),
                    name(MoveSlot::Tilt)
                )
            })?;
            *from = from_written.target(MoveEnd::From)?;
        }
        EffectType::Rainbow {
            speed,
            saturation,
            brightness,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "saturation" => {
                        let val = parse_level(key, value)?;
                        *saturation = val;
                    }
                    "brightness" => {
                        let val = parse_level(key, value)?;
                        *brightness = val;
                    }
                    "duration" => *duration = ctx.duration(value)?,
                    other => return Err(unread(other).into()),
                }
            }
        }
    }

    ignored.sort();
    Ok((effect_type, ignored))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default ParseContext used by unit tests that don't need tempo/offset.
    fn default_ctx() -> ParseContext {
        ParseContext {
            tempo_map: None,
            cue_time: Duration::ZERO,
            offset_secs: 0.0,
        }
    }

    // ── color_to_normalized_params ─────────────────────────────────

    #[test]
    fn color_normalize_white() {
        let c = Color {
            r: 255,
            g: 255,
            b: 255,
            w: None,
        };
        let (r, g, b) = color_to_normalized_params(&c);
        assert!((r - 1.0).abs() < 1e-9);
        assert!((g - 1.0).abs() < 1e-9);
        assert!((b - 1.0).abs() < 1e-9);
    }

    #[test]
    fn color_normalize_black() {
        let c = Color {
            r: 0,
            g: 0,
            b: 0,
            w: None,
        };
        let (r, g, b) = color_to_normalized_params(&c);
        assert!((r - 0.0).abs() < 1e-9);
        assert!((g - 0.0).abs() < 1e-9);
        assert!((b - 0.0).abs() < 1e-9);
    }

    #[test]
    fn color_normalize_half() {
        let c = Color {
            r: 128,
            g: 64,
            b: 0,
            w: None,
        };
        let (r, g, b) = color_to_normalized_params(&c);
        assert!((r - 128.0 / 255.0).abs() < 1e-9);
        assert!((g - 64.0 / 255.0).abs() < 1e-9);
        assert!((b - 0.0).abs() < 1e-9);
    }

    // ── effect_with_defaults / effect_keyword ──────────────────────

    #[test]
    fn every_effect_keyword_round_trips() {
        for keyword in [
            "static", "cycle", "strobe", "pulse", "chase", "dimmer", "rainbow", "move",
        ] {
            let effect = effect_with_defaults(keyword).expect(keyword);
            assert_eq!(effect_keyword(&effect), keyword);
        }
        assert!(effect_with_defaults("flicker").is_none());
    }

    // ── clean_string_value ─────────────────────────────────────────

    #[test]
    fn clean_strips_quotes_and_lowercases() {
        assert_eq!(clean_string_value("\"Forward\""), "forward");
    }

    #[test]
    fn clean_trims_whitespace() {
        assert_eq!(clean_string_value("  hello  "), "hello");
    }

    #[test]
    fn clean_no_quotes() {
        assert_eq!(clean_string_value("snap"), "snap");
    }

    #[test]
    fn clean_mixed() {
        assert_eq!(clean_string_value("\"  PingPong  \""), "pingpong");
    }

    // ── apply_parameters_to_effect_type — Static ───────────────────

    #[test]
    fn apply_static_dimmer() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("dimmer".to_string(), "50%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["dimmer"] - 0.5).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    /// `intensity` is documented as a common parameter applying to all effects, and
    /// used to be dropped on the floor by static's catch-all (it isn't a bare f64).
    /// Regression test for #346.
    #[test]
    fn apply_static_intensity_is_a_synonym_for_dimmer() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("intensity".to_string(), "22%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["dimmer"] - 0.22).abs() < 1e-9);
            assert!(!parameters.contains_key("intensity"));
        } else {
            panic!("Expected Static");
        }
    }

    #[test]
    fn apply_static_color() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("color".to_string(), "#FF8000".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["red"] - 1.0).abs() < 1e-9);
            assert!((parameters["green"] - 128.0 / 255.0).abs() < 1e-2);
            assert!((parameters["blue"] - 0.0).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    #[test]
    fn apply_static_rgb_channels() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("red".to_string(), "100%".to_string());
        params.insert("green".to_string(), "50%".to_string());
        params.insert("blue".to_string(), "0%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["red"] - 1.0).abs() < 1e-9);
            assert!((parameters["green"] - 0.5).abs() < 1e-9);
            assert!((parameters["blue"] - 0.0).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    #[test]
    fn apply_static_duration() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("duration".to_string(), "2s".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Static { duration, .. } = result {
            assert_eq!(duration, Duration::from_secs(2));
        } else {
            panic!("Expected Static");
        }
    }

    // ── apply_parameters_to_effect_type — ColorCycle ───────────────

    #[test]
    fn apply_color_cycle_colors() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let colors = vec!["red".to_string(), "#0000FF".to_string()];
        let result = apply_parameters_to_effect_type(et, &HashMap::new(), &colors, &default_ctx())
            .unwrap()
            .0;
        if let EffectType::ColorCycle { colors, .. } = result {
            assert_eq!(colors.len(), 2);
            assert_eq!(colors[0].r, 255);
            assert_eq!(colors[1].b, 255);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    #[test]
    fn apply_color_cycle_direction() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "backward".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::ColorCycle { direction, .. } = result {
            assert_eq!(direction, CycleDirection::Backward);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    #[test]
    fn apply_color_cycle_transition_crossfade() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "crossfade".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::ColorCycle { transition, .. } = result {
            assert_eq!(transition, CycleTransition::Fade);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    // ── apply_parameters_to_effect_type — Strobe ───────────────────

    #[test]
    fn apply_strobe_frequency() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("frequency".to_string(), "15.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Strobe { frequency, .. } = result {
            assert_eq!(frequency, TempoAwareFrequency::Fixed(15.0));
        } else {
            panic!("Expected Strobe");
        }
    }

    #[test]
    fn apply_strobe_rate_alias() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("rate".to_string(), "20.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Strobe { frequency, .. } = result {
            assert_eq!(frequency, TempoAwareFrequency::Fixed(20.0));
        } else {
            panic!("Expected Strobe");
        }
    }

    // ── apply_parameters_to_effect_type — Pulse ────────────────────

    #[test]
    fn apply_pulse_params() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("base_level".to_string(), "20%".to_string());
        params.insert("intensity".to_string(), "80%".to_string());
        params.insert("frequency".to_string(), "4.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Pulse {
            base_level,
            pulse_amplitude,
            frequency,
            ..
        } = result
        {
            assert!((base_level - 0.2).abs() < 1e-9);
            assert!((pulse_amplitude - 0.8).abs() < 1e-9);
            assert_eq!(frequency, TempoAwareFrequency::Fixed(4.0));
        } else {
            panic!("Expected Pulse");
        }
    }

    // ── apply_parameters_to_effect_type — Chase ────────────────────

    #[test]
    fn apply_chase_params() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pattern".to_string(), "snake".to_string());
        params.insert("direction".to_string(), "clockwise".to_string());
        params.insert("transition".to_string(), "fade".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Chase {
            pattern,
            direction,
            transition,
            ..
        } = result
        {
            assert_eq!(pattern, ChasePattern::Snake);
            assert!(matches!(direction, ChaseDirection::Clockwise));
            assert_eq!(transition, CycleTransition::Fade);
        } else {
            panic!("Expected Chase");
        }
    }

    // ── apply_parameters_to_effect_type — Dimmer ───────────────────

    #[test]
    fn apply_dimmer_params() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("start".to_string(), "25%".to_string());
        params.insert("end".to_string(), "75%".to_string());
        params.insert("duration".to_string(), "3s".to_string());
        params.insert("curve".to_string(), "exponential".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Dimmer {
            start_level,
            end_level,
            duration,
            curve,
        } = result
        {
            assert!((start_level - 0.25).abs() < 1e-9);
            assert!((end_level - 0.75).abs() < 1e-9);
            assert_eq!(duration, Duration::from_secs(3));
            assert!(matches!(curve, DimmerCurve::Exponential));
        } else {
            panic!("Expected Dimmer");
        }
    }

    #[test]
    fn apply_dimmer_invalid_curve() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("curve".to_string(), "invalid_curve".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── apply_parameters_to_effect_type — Rainbow ──────────────────

    #[test]
    fn apply_rainbow_params() {
        let et = EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("saturation".to_string(), "80%".to_string());
        params.insert("brightness".to_string(), "60%".to_string());
        params.insert("speed".to_string(), "2.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Rainbow {
            speed,
            saturation,
            brightness,
            ..
        } = result
        {
            assert_eq!(speed, TempoAwareSpeed::Fixed(2.0));
            assert!((saturation - 0.8).abs() < 1e-9);
            assert!((brightness - 0.6).abs() < 1e-9);
        } else {
            panic!("Expected Rainbow");
        }
    }

    // ── Error cases ────────────────────────────────────────────────

    #[test]
    fn apply_invalid_direction_error() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "sideways".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_invalid_chase_direction_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "diagonal".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_invalid_transition_error() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "dissolve".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Dimmer curve variants ───────────────────────────────────────

    #[test]
    fn apply_dimmer_curve_logarithmic() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("curve".to_string(), "logarithmic".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Dimmer { curve, .. } = result {
            assert!(matches!(curve, DimmerCurve::Logarithmic));
        } else {
            panic!("Expected Dimmer");
        }
    }

    // ── Cycle direction PingPong ─────────────────────────────────────

    #[test]
    fn apply_color_cycle_direction_pingpong() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "pingpong".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::ColorCycle { direction, .. } = result {
            assert_eq!(direction, CycleDirection::PingPong);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    // ── Chase direction variants ─────────────────────────────────────

    #[test]
    fn apply_chase_direction_right_to_left() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "right_to_left".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Chase { direction, .. } = result {
            assert!(matches!(direction, ChaseDirection::RightToLeft));
        } else {
            panic!("Expected Chase");
        }
    }

    // ── Chase pattern random ─────────────────────────────────────────

    #[test]
    fn apply_chase_pattern_random() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pattern".to_string(), "random".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Chase { pattern, .. } = result {
            assert!(matches!(pattern, ChasePattern::Random));
        } else {
            panic!("Expected Chase");
        }
    }

    // ── Invalid chase pattern ────────────────────────────────────────

    #[test]
    fn apply_invalid_chase_pattern_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pattern".to_string(), "invalid_pattern".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Chase transition crossfade alias ────────────────────────────

    #[test]
    fn apply_chase_transition_crossfade() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "crossfade".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Chase { transition, .. } = result {
            assert_eq!(transition, CycleTransition::Fade);
        } else {
            panic!("Expected Chase");
        }
    }

    #[test]
    fn hue_transition_is_for_cycles_only() {
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "hue".to_string());
        let cycle = EffectType::ColorCycle {
            colors: vec![],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let result = apply_parameters_to_effect_type(cycle, &params, &[], &default_ctx())
            .unwrap()
            .0;
        assert!(matches!(
            result,
            EffectType::ColorCycle {
                transition: CycleTransition::Hue,
                ..
            }
        ));

        let chase = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        assert!(apply_parameters_to_effect_type(chase, &params, &[], &default_ctx()).is_err());
    }

    #[test]
    fn apply_invalid_chase_transition_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "wipe".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Static effect with generic key fallback ──────────────────────

    #[test]
    fn apply_static_unknown_key_numeric() {
        // Unknown key with numeric value should still be stored
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("custom_param".to_string(), "0.75".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["custom_param"] - 0.75).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    // ── Dimmer start_level alias ──────────────────────────────────────

    #[test]
    fn apply_dimmer_start_level_alias() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("start_level".to_string(), "30%".to_string());
        params.insert("end_level".to_string(), "90%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Dimmer {
            start_level,
            end_level,
            ..
        } = result
        {
            assert!((start_level - 0.3).abs() < 1e-9);
            assert!((end_level - 0.9).abs() < 1e-9);
        } else {
            panic!("Expected Dimmer");
        }
    }

    // ── Pulse duration ───────────────────────────────────────────────

    #[test]
    fn apply_pulse_duration() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("duration".to_string(), "5s".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Pulse { duration, .. } = result {
            assert_eq!(duration, Duration::from_secs(5));
        } else {
            panic!("Expected Pulse");
        }
    }

    // ── Strobe duration ──────────────────────────────────────────────

    #[test]
    fn apply_strobe_duration() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("duration".to_string(), "3s".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Strobe { duration, .. } = result {
            assert_eq!(duration, Duration::from_secs(3));
        } else {
            panic!("Expected Strobe");
        }
    }

    // ── Color cycle speed ────────────────────────────────────────────

    #[test]
    fn apply_color_cycle_speed() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "2.5".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::ColorCycle { speed, .. } = result {
            assert_eq!(speed, TempoAwareSpeed::Fixed(2.5));
        } else {
            panic!("Expected ColorCycle");
        }
    }

    // ── Chase speed ──────────────────────────────────────────────────

    #[test]
    fn apply_chase_speed() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "3.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Chase { speed, .. } = result {
            assert_eq!(speed, TempoAwareSpeed::Fixed(3.0));
        } else {
            panic!("Expected Chase");
        }
    }

    // ── Invalid speed errors ────────────────────────────────────────

    #[test]
    fn apply_color_cycle_invalid_speed_error() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "not_a_number".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_chase_invalid_speed_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "invalid".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_rainbow_invalid_speed_error() {
        let et = EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "bad".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Invalid frequency errors ────────────────────────────────────

    #[test]
    fn apply_strobe_invalid_frequency_error() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("frequency".to_string(), "not_valid".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_pulse_invalid_frequency_error() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("frequency".to_string(), "xyz".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Pulse pulse_amplitude alias ─────────────────────────────────

    #[test]
    fn apply_pulse_pulse_amplitude_key() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pulse_amplitude".to_string(), "70%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx())
            .unwrap()
            .0;
        if let EffectType::Pulse {
            pulse_amplitude, ..
        } = result
        {
            assert!((pulse_amplitude - 0.7).abs() < 1e-9);
        } else {
            panic!("Expected Pulse");
        }
    }
}
