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
//! A `.light` file as written: the web UI's editing model, read off the
//! one grammar (`grammar.pest`) so the editor and the player agree on what
//! a line says. Nothing here is resolved -- a `@bar/beat` stays a bar and
//! a beat, a `2beats` stays the text `2beats`, a parameter is its name and
//! its value as written -- so the editor's serializer can write the file
//! back; where a line lands in time is [`super::layout`]'s business.
//!
//! The types' TypeScript declarations are generated from them (`tsify`)
//! for the web UI, which holds and edits a [`Document`] and never parses
//! the DSL itself.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

use pest::iterators::Pair;
use pest::Parser;
use serde::Serialize;

use super::grammar::{LightingParser, Rule};
use super::include::normalize;
use super::utils::{items, parse_measure_time, parse_time_string};

/// A `.light` file as written.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Document {
    /// Its `include`s, in order, when it has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub includes: Option<Vec<Include>>,
    /// Its file-level `tempo` block (the last, if it has several: the one
    /// the player takes).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub tempo: Option<TempoSection>,
    pub sequences: Vec<Sequence>,
    pub shows: Vec<LightShow>,
}

/// What reading a file as a [`Document`] gives: the document, or why the
/// editor cannot hold the file (it does not parse, or it defines what the
/// editor does not edit).
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DocumentResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub document: Option<Document>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub error: Option<String>,
}

/// An `include "…"` as written, and the file it names.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Include {
    /// The path as written, relative to the file's folder.
    pub written: String,
    /// The included file's path, resolved against the file's own path
    /// when one is given (else as written): what to fetch it by.
    pub path: String,
    pub line: u32,
}

/// A `tempo { }` block as written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoSection {
    pub start: DurationValue,
    pub bpm: f64,
    pub time_signature: (u32, u32),
    pub changes: Vec<TempoChange>,
}

/// One entry of a tempo block's `changes:`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoChange {
    pub timestamp: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub bpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub time_signature: Option<(u32, u32)>,
    /// `transition:` as written: `4`, `2m` or `snap`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub transition: Option<String>,
}

/// A number with its unit, as a tempo block's `start:` is written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DurationValue {
    pub value: f64,
    pub unit: DurationUnit,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DurationUnit {
    Ms,
    S,
    Beats,
    Measures,
}

/// A cue's or a tempo change's position: a time (`@MM:SS.mmm`, `@S.sss`)
/// in milliseconds, or a bar and beat (`@bar/beat`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Timestamp {
    #[serde(rename = "type")]
    pub kind: TimestampKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub measure: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub beat: Option<f64>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TimestampKind {
    Absolute,
    MeasureBeat,
}

/// A `sequence "name"(params), length: … { }` definition.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Sequence {
    pub name: String,
    /// Its parameters (`$name` in the body), each with its default if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub params: Option<Vec<SequenceArg>>,
    /// Its `length:` as written, e.g. `1measure`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub length: Option<String>,
    pub cues: Vec<Cue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub line: Option<u32>,
}

/// A `name: value` in a sequence's parameter list or a reference's
/// arguments; a declared parameter with no default is its name alone.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SequenceArg {
    pub name: String,
    /// As written: a quoted colour, a group, a number, a `$param`…
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub value: Option<String>,
}

/// A `show "name" { }`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LightShow {
    /// As written; `""` for a show written without a name (the only show
    /// of a file may be, and plays as `default`).
    pub name: String,
    pub cues: Vec<Cue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub line: Option<u32>,
}

/// One `@time` and everything written under it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cue {
    pub timestamp: Timestamp,
    pub effects: Vec<CueEffect>,
    pub commands: Vec<LayerCommand>,
    pub sequences: Vec<SequenceRef>,
    /// The comment lines written above it, their markers stripped.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// What its `offset N measures` commands add up to, when it has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub offset_measures: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub reset_measures: Option<bool>,
    /// Its inline `loop { } repeats: n` blocks, as written: the editor
    /// keeps them as text.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loops: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub line: Option<u32>,
}

/// An effect line: its groups and its parameters.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CueEffect {
    pub groups: Vec<String>,
    pub effect: EffectParams,
}

/// An effect's parameters as written: the ones the editor has controls
/// for by name, numbers read as numbers (one that is not a plain number --
/// `50%`, `$level` -- stays as written in `extra`), the rest under
/// `extra` by name.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EffectParams {
    #[serde(rename = "type")]
    pub kind: EffectKind,
    /// Each `color:`, unquoted, in order.
    pub colors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub intensity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub dimmer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub frequency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub transition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub up_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub hold_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub down_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub start_level: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub end_level: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub curve: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub blend_mode: Option<String>,
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loop_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub base_level: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub pulse_amplitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub saturation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub brightness: Option<f64>,
    /// `move`: the focus point to aim at (`focus:` or `to:`), unquoted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub focus: Option<String>,
    /// `move`: the focus point to start from, unquoted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub from: Option<String>,
    /// `move`: explicit angles, with their unit (`45deg`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub pan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub tilt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub from_pan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub from_tilt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub easing: Option<String>,
    /// Every other parameter, by name, as written.
    pub extra: BTreeMap<String, String>,
}

/// The effect keywords of the grammar.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Static,
    Cycle,
    Strobe,
    Pulse,
    Chase,
    Dimmer,
    Rainbow,
    Move,
}

/// A `clear(…)`, `freeze(…)`, `unfreeze(…)` or `master(…)`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LayerCommand {
    pub command: LayerCommandKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub intensity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub speed: Option<String>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LayerCommandKind {
    Clear,
    Freeze,
    Unfreeze,
    Master,
}

/// A `sequence "name"(args), loop: …` reference, or a `stop sequence
/// "name"`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SequenceRef {
    pub name: String,
    /// The arguments bound here, in order, values as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub args: Option<Vec<SequenceArg>>,
    /// `loop:` as written: a count, `loop`, `once`, or a `$param`.
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loop_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub stop: Option<bool>,
}

/// Reads `source` as a [`Document`]. `path` is where the file is, which
/// its `include`s resolve against. A file the editor cannot hold -- one
/// that does not parse, defines a fixture type or a venue, or gives a show
/// or a sequence a `tempo` block of its own -- is an error, so that
/// writing the document back can never drop what the file said.
pub fn document(source: &str, path: Option<&Path>) -> DocumentResult {
    match read(source, path) {
        Ok(document) => DocumentResult {
            document: Some(document),
            error: None,
        },
        Err(error) => DocumentResult {
            document: None,
            error: Some(error.to_string()),
        },
    }
}

fn read(source: &str, path: Option<&Path>) -> Result<Document, Box<dyn Error>> {
    let pairs = LightingParser::parse(Rule::file, source)
        .map_err(|e| super::error::describe_pest_error(&e, source))?;
    let folder = path.and_then(Path::parent).unwrap_or(Path::new(""));
    let mut document = Document::default();
    let mut includes = Vec::new();
    for pair in items(pairs) {
        let line = pair.line_col().0 as u32;
        match pair.as_rule() {
            Rule::include => {
                let written = pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::string)
                    .map(|p| p.as_str().trim_matches('"').to_string())
                    .unwrap_or_default();
                let path = normalize(&folder.join(&written))
                    .to_string_lossy()
                    .into_owned();
                includes.push(Include {
                    written,
                    path,
                    line,
                });
            }
            Rule::tempo => document.tempo = Some(tempo_section(pair)?),
            Rule::sequence => document.sequences.push(sequence(pair, source)?),
            Rule::light_show => document.shows.push(show(pair, source)?),
            Rule::fixture_type | Rule::venue => {
                let what = if pair.as_rule() == Rule::venue {
                    "a venue"
                } else {
                    "a fixture type"
                };
                return Err(format!(
                    "line {line}: this file defines {what}, which the show editor does not edit"
                )
                .into());
            }
            _ => {}
        }
    }
    if !includes.is_empty() {
        document.includes = Some(includes);
    }
    Ok(document)
}

fn tempo_section(pair: Pair<Rule>) -> Result<TempoSection, Box<dyn Error>> {
    let mut section = TempoSection {
        start: DurationValue {
            value: 0.0,
            unit: DurationUnit::S,
        },
        bpm: crate::tempo::DEFAULT_BPM,
        time_signature: (
            crate::tempo::DEFAULT_TIME_SIGNATURE.numerator,
            crate::tempo::DEFAULT_TIME_SIGNATURE.denominator,
        ),
        changes: Vec::new(),
    };
    let content = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::tempo_content)
        .into_iter()
        .flat_map(Pair::into_inner);
    for item in content {
        match item.as_rule() {
            Rule::tempo_start => {
                if let Some(value) = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::time_parameter)
                {
                    section.start = duration_value(value.as_str())?;
                }
            }
            Rule::tempo_bpm => {
                if let Some(value) = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::number_value)
                {
                    section.bpm = value.as_str().trim().parse()?;
                }
            }
            Rule::tempo_time_signature => {
                if let Some(value) = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::time_sig_value)
                {
                    section.time_signature = signature(value.as_str())?;
                }
            }
            Rule::tempo_changes => {
                for change in item
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::tempo_change_list)
                    .flat_map(Pair::into_inner)
                    .filter(|p| p.as_rule() == Rule::tempo_change)
                {
                    section.changes.push(tempo_change(change)?);
                }
            }
            _ => {}
        }
    }
    Ok(section)
}

fn tempo_change(pair: Pair<Rule>) -> Result<TempoChange, Box<dyn Error>> {
    let mut change = TempoChange {
        timestamp: Timestamp {
            kind: TimestampKind::Absolute,
            ms: Some(0.0),
            measure: None,
            beat: None,
        },
        bpm: None,
        time_signature: None,
        transition: None,
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::time_string | Rule::measure_time => change.timestamp = timestamp(item)?,
            Rule::tempo_change_content => {
                for param in item
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::tempo_change_param)
                    .flat_map(Pair::into_inner)
                {
                    let rule = param.as_rule();
                    let Some(value) = param.into_inner().next() else {
                        continue;
                    };
                    match rule {
                        Rule::tempo_change_bpm => change.bpm = Some(value.as_str().trim().parse()?),
                        Rule::tempo_change_time_signature => {
                            change.time_signature = Some(signature(value.as_str())?)
                        }
                        Rule::tempo_change_transition => {
                            change.transition = Some(value.as_str().trim().to_string())
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(change)
}

/// `4/4` as `(4, 4)`, as written: the player's own range check is in
/// `parse_signature_in`, applied when the file loads.
fn signature(raw: &str) -> Result<(u32, u32), Box<dyn Error>> {
    let (numerator, denominator) = raw
        .trim()
        .split_once('/')
        .ok_or_else(|| format!("Invalid time signature: {raw}"))?;
    Ok((numerator.trim().parse()?, denominator.trim().parse()?))
}

/// `2.5s`, `500ms`, `1beat`, `2measures` as the number and its unit.
fn duration_value(raw: &str) -> Result<DurationValue, Box<dyn Error>> {
    let (number, unit) = super::utils::split_unit(raw);
    let unit = match unit {
        "ms" => DurationUnit::Ms,
        "s" | "" => DurationUnit::S,
        "beat" | "beats" => DurationUnit::Beats,
        "measure" | "measures" => DurationUnit::Measures,
        other => return Err(format!("Unknown time unit: {other}").into()),
    };
    Ok(DurationValue {
        value: number.trim().parse()?,
        unit,
    })
}

fn timestamp(pair: Pair<Rule>) -> Result<Timestamp, Box<dyn Error>> {
    Ok(match pair.as_rule() {
        Rule::measure_time => {
            let (measure, beat) = parse_measure_time(pair.as_str())?;
            Timestamp {
                kind: TimestampKind::MeasureBeat,
                ms: None,
                measure: Some(measure),
                beat: Some(beat),
            }
        }
        // Whole milliseconds, the parser's own precision: a fraction of a
        // millisecond is never written, and `1.005` as seconds times a
        // thousand is not 1005.
        _ => Timestamp {
            kind: TimestampKind::Absolute,
            ms: Some(parse_time_string(pair.as_str())?.as_millis() as f64),
            measure: None,
            beat: None,
        },
    })
}

fn sequence(pair: Pair<Rule>, source: &str) -> Result<Sequence, Box<dyn Error>> {
    let line = pair.line_col().0 as u32;
    let mut sequence = Sequence {
        name: String::new(),
        params: None,
        length: None,
        cues: Vec::new(),
        line: Some(line),
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::sequence_name => sequence.name = item.as_str().trim_matches('"').to_string(),
            Rule::sequence_param_decls => {
                sequence.params = Some(
                    item.into_inner()
                        .filter(|p| p.as_rule() == Rule::sequence_param_decl)
                        .map(arg)
                        .collect(),
                )
            }
            Rule::sequence_length => {
                sequence.length = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::time_parameter)
                    .map(|p| p.as_str().trim().to_string())
            }
            Rule::sequence_content => sequence.cues = cues(item, source, "a sequence")?,
            _ => {}
        }
    }
    Ok(sequence)
}

fn show(pair: Pair<Rule>, source: &str) -> Result<LightShow, Box<dyn Error>> {
    let line = pair.line_col().0 as u32;
    let mut show = LightShow {
        name: String::new(),
        cues: Vec::new(),
        line: Some(line),
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::show_name => show.name = item.as_str().trim_matches('"').to_string(),
            Rule::show_content => show.cues = cues(item, source, "a show")?,
            _ => {}
        }
    }
    Ok(show)
}

/// A `name: value` (or a bare `name`) of a parameter list.
fn arg(pair: Pair<Rule>) -> SequenceArg {
    let mut parts = pair.into_inner();
    SequenceArg {
        name: parts
            .next()
            .map(|p| p.as_str().trim().to_string())
            .unwrap_or_default(),
        value: parts.next().map(|p| p.as_str().trim().to_string()),
    }
}

/// The cues of a show's or a sequence's body, each with the comment lines
/// above it.
fn cues(content: Pair<Rule>, source: &str, what: &str) -> Result<Vec<Cue>, Box<dyn Error>> {
    // The body's text begins at its `{`; comment lines are looked for
    // from the line after the one a cue (or the brace) ends on.
    let mut previous_end = content.as_span().start();
    let mut cues = Vec::new();
    for item in content.into_inner() {
        match item.as_rule() {
            Rule::tempo => {
                return Err(format!(
                    "line {}: {what} with a tempo block of its own cannot be edited on the \
                     timeline; edit the text",
                    item.line_col().0
                )
                .into())
            }
            Rule::cue | Rule::sequence_cue => {
                let span = item.as_span();
                let comment = comment_lines(source, previous_end, span.start());
                previous_end = span.end();
                let mut cue = cue(item)?;
                cue.comment = comment;
                cues.push(cue);
            }
            _ => {}
        }
    }
    Ok(cues)
}

/// The comment lines between the line after the one `from` is on and
/// `to`, their markers stripped, joined by newlines; `None` when there
/// are none.
fn comment_lines(source: &str, from: usize, to: usize) -> Option<String> {
    let start = source[from..to].find('\n').map(|i| from + i + 1)?;
    let lines: Vec<&str> = source[start..to]
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_prefix('#')
                .or_else(|| line.strip_prefix("//"))
                .map(str::trim)
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn cue(pair: Pair<Rule>) -> Result<Cue, Box<dyn Error>> {
    let line = pair.line_col().0 as u32;
    let mut cue = Cue {
        timestamp: Timestamp {
            kind: TimestampKind::Absolute,
            ms: Some(0.0),
            measure: None,
            beat: None,
        },
        effects: Vec::new(),
        commands: Vec::new(),
        sequences: Vec::new(),
        comment: None,
        offset_measures: None,
        reset_measures: None,
        loops: None,
        line: Some(line),
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::time_string | Rule::measure_time => cue.timestamp = timestamp(item)?,
            Rule::effect => cue.effects.push(effect(item)?),
            Rule::layer_command => cue.commands.push(layer_command(item)),
            Rule::sequence_reference => cue.sequences.push(sequence_ref(item)),
            Rule::stop_sequence_command => {
                let name = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::sequence_name)
                    .map(|p| p.as_str().trim_matches('"').to_string())
                    .unwrap_or_default();
                cue.sequences.push(SequenceRef {
                    name,
                    args: None,
                    loop_: None,
                    stop: Some(true),
                });
            }
            Rule::offset_command => {
                let measures: u32 = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::number_value)
                    .map(|p| p.as_str().trim().parse())
                    .transpose()?
                    .unwrap_or(0);
                cue.offset_measures = Some(cue.offset_measures.unwrap_or(0) + measures);
            }
            Rule::reset_measures_command => cue.reset_measures = Some(true),
            Rule::inline_loop => cue
                .loops
                .get_or_insert_with(Vec::new)
                .push(item.as_str().to_string()),
            _ => {}
        }
    }
    Ok(cue)
}

fn effect(pair: Pair<Rule>) -> Result<CueEffect, Box<dyn Error>> {
    let mut groups = Vec::new();
    let mut params = EffectParams {
        kind: EffectKind::Static,
        colors: Vec::new(),
        intensity: None,
        dimmer: None,
        frequency: None,
        speed: None,
        direction: None,
        transition: None,
        duration: None,
        up_time: None,
        hold_time: None,
        down_time: None,
        start_level: None,
        end_level: None,
        curve: None,
        layer: None,
        blend_mode: None,
        loop_: None,
        base_level: None,
        pulse_amplitude: None,
        pattern: None,
        saturation: None,
        brightness: None,
        focus: None,
        from: None,
        pan: None,
        tilt: None,
        from_pan: None,
        from_tilt: None,
        easing: None,
        extra: BTreeMap::new(),
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::group_list => {
                groups.extend(item.into_inner().map(|g| g.as_str().trim().to_string()))
            }
            Rule::effect_type => {
                params.kind = match item.as_str() {
                    "static" => EffectKind::Static,
                    "cycle" => EffectKind::Cycle,
                    "strobe" => EffectKind::Strobe,
                    "pulse" => EffectKind::Pulse,
                    "chase" => EffectKind::Chase,
                    "dimmer" => EffectKind::Dimmer,
                    "rainbow" => EffectKind::Rainbow,
                    "move" => EffectKind::Move,
                    other => return Err(format!("Unknown effect type: {other}").into()),
                }
            }
            Rule::parameters => {
                for parameter in item.into_inner().filter(|p| p.as_rule() == Rule::parameter) {
                    let mut parts = parameter.into_inner();
                    let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let name = name.as_str().trim().trim_matches('"').to_string();
                    let value = value.as_str().trim();
                    set_param(&mut params, &name, value);
                }
            }
            _ => {}
        }
    }
    Ok(CueEffect {
        groups,
        effect: params,
    })
}

/// Files a parameter under the field the editor edits it by, else under
/// `extra`.
fn set_param(params: &mut EffectParams, name: &str, value: &str) {
    let text = || value.to_string();
    let unquoted = || value.trim_matches('"').to_string();
    let number =
        |slot: &mut Option<f64>, extra: &mut BTreeMap<String, String>| match value.parse::<f64>() {
            Ok(n) => *slot = Some(n),
            Err(_) => {
                extra.insert(name.to_string(), value.to_string());
            }
        };
    match name {
        "color" => params.colors.push(unquoted()),
        "intensity" => number(&mut params.intensity, &mut params.extra),
        "dimmer" => params.dimmer = Some(text()),
        "frequency" => params.frequency = Some(text()),
        "speed" => params.speed = Some(text()),
        "direction" => params.direction = Some(text()),
        "transition" => params.transition = Some(text()),
        "duration" => params.duration = Some(text()),
        "up_time" => params.up_time = Some(text()),
        "hold_time" => params.hold_time = Some(text()),
        "down_time" => params.down_time = Some(text()),
        "start_level" => number(&mut params.start_level, &mut params.extra),
        "end_level" => number(&mut params.end_level, &mut params.extra),
        "curve" => params.curve = Some(text()),
        "layer" => params.layer = Some(text()),
        "blend_mode" => params.blend_mode = Some(text()),
        "loop" => params.loop_ = Some(text()),
        "base_level" => number(&mut params.base_level, &mut params.extra),
        "pulse_amplitude" => number(&mut params.pulse_amplitude, &mut params.extra),
        "pattern" => params.pattern = Some(text()),
        "saturation" => number(&mut params.saturation, &mut params.extra),
        "brightness" => number(&mut params.brightness, &mut params.extra),
        "focus" | "to" => params.focus = Some(unquoted()),
        "from" => params.from = Some(unquoted()),
        "pan" => params.pan = Some(text()),
        "tilt" => params.tilt = Some(text()),
        "from_pan" => params.from_pan = Some(text()),
        "from_tilt" => params.from_tilt = Some(text()),
        "easing" => params.easing = Some(text()),
        _ => {
            params.extra.insert(name.to_string(), text());
        }
    }
}

fn layer_command(pair: Pair<Rule>) -> LayerCommand {
    let mut command = LayerCommand {
        command: LayerCommandKind::Clear,
        layer: None,
        intensity: None,
        speed: None,
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::layer_command_type => {
                command.command = match item.as_str() {
                    "freeze" => LayerCommandKind::Freeze,
                    "unfreeze" => LayerCommandKind::Unfreeze,
                    "master" => LayerCommandKind::Master,
                    _ => LayerCommandKind::Clear,
                }
            }
            Rule::layer_command_params => {
                for param in item
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::layer_command_param)
                {
                    let mut parts = param.into_inner();
                    let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let value = Some(value.as_str().trim().to_string());
                    match name.as_str().trim() {
                        "layer" => command.layer = value,
                        "intensity" => command.intensity = value,
                        "speed" => command.speed = value,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    command
}

fn sequence_ref(pair: Pair<Rule>) -> SequenceRef {
    let mut reference = SequenceRef {
        name: String::new(),
        args: None,
        loop_: None,
        stop: None,
    };
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::sequence_name => reference.name = item.as_str().trim_matches('"').to_string(),
            Rule::sequence_args => {
                reference.args = Some(
                    item.into_inner()
                        .filter(|p| p.as_rule() == Rule::sequence_arg)
                        .map(arg)
                        .collect(),
                )
            }
            Rule::sequence_params => {
                for param in item
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::sequence_param)
                {
                    let mut parts = param.into_inner();
                    let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    if name.as_str().trim() == "loop" {
                        reference.loop_ = Some(value.as_str().trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    reference
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOW: &str = r##"include "../common/looks.light"

tempo {
    start: 0.5s
    bpm: 120
    time_signature: 4/4
    changes: [
        @3/1 { bpm: 180, transition: 8 },
        @00:10.000 { time_signature: 3/4 }
    ]
}

sequence "strike"(hit, bed: "#0000FF"), length: 1measure {
    @1/1
    front: static, color: $hit, duration: 1beat
}

show "Song" {
    # The opening look
    # held for a bar
    @1/1
    front, back: static, color: "blue", intensity: 0.6, dimmer: 80%, duration: 1measure, layer: foreground
    clear(layer: background)
    sequence "strike"(hit: "#FF0000"), loop: 2
    offset 2 measures

    // a stop
    @2/1.5
    stop sequence "strike"
    reset_measures
    movers: move, focus: "front", easing: smooth, duration: 2s
    loop {
        @0.0
        front: strobe, frequency: 4, duration: 0.5s
    } repeats: 3
}
"##;

    #[test]
    fn reads_the_file_as_written() {
        let doc = document(SHOW, Some(Path::new("band/song/show.light")))
            .document
            .expect("parses");
        let include = &doc.includes.as_ref().expect("include")[0];
        assert_eq!(include.written, "../common/looks.light");
        assert_eq!(include.path, "band/common/looks.light");
        let tempo = doc.tempo.expect("tempo");
        assert_eq!(tempo.start.value, 0.5);
        assert_eq!(tempo.start.unit, DurationUnit::S);
        assert_eq!(tempo.bpm, 120.0);
        assert_eq!(tempo.changes.len(), 2);
        assert_eq!(tempo.changes[0].timestamp.measure, Some(3));
        assert_eq!(tempo.changes[0].transition.as_deref(), Some("8"));
        assert_eq!(tempo.changes[1].timestamp.ms, Some(10000.0));
        assert_eq!(tempo.changes[1].time_signature, Some((3, 4)));
        let seq = &doc.sequences[0];
        assert_eq!(seq.name, "strike");
        assert_eq!(seq.length.as_deref(), Some("1measure"));
        let params = seq.params.as_ref().expect("params");
        assert_eq!(params[0].name, "hit");
        assert_eq!(params[0].value, None);
        assert_eq!(params[1].value.as_deref(), Some("\"#0000FF\""));
        assert_eq!(seq.cues[0].effects[0].effect.colors, vec!["$hit"]);
        let show = &doc.shows[0];
        assert_eq!(show.name, "Song");
        let first = &show.cues[0];
        assert_eq!(
            first.comment.as_deref(),
            Some("The opening look\nheld for a bar")
        );
        assert_eq!(first.timestamp.kind, TimestampKind::MeasureBeat);
        let effect = &first.effects[0];
        assert_eq!(effect.groups, vec!["front", "back"]);
        assert_eq!(effect.effect.kind, EffectKind::Static);
        assert_eq!(effect.effect.colors, vec!["blue"]);
        assert_eq!(effect.effect.intensity, Some(0.6));
        assert_eq!(effect.effect.dimmer.as_deref(), Some("80%"));
        assert_eq!(effect.effect.duration.as_deref(), Some("1measure"));
        assert_eq!(effect.effect.layer.as_deref(), Some("foreground"));
        assert_eq!(first.commands[0].command, LayerCommandKind::Clear);
        assert_eq!(first.commands[0].layer.as_deref(), Some("background"));
        assert_eq!(first.sequences[0].name, "strike");
        assert_eq!(first.sequences[0].loop_.as_deref(), Some("2"));
        assert_eq!(
            first.sequences[0].args.as_ref().expect("args")[0]
                .value
                .as_deref(),
            Some("\"#FF0000\"")
        );
        assert_eq!(first.offset_measures, Some(2));
        let second = &show.cues[1];
        assert_eq!(second.comment.as_deref(), Some("a stop"));
        assert_eq!(second.timestamp.beat, Some(1.5));
        assert_eq!(second.sequences[0].stop, Some(true));
        assert_eq!(second.reset_measures, Some(true));
        assert_eq!(second.effects[0].effect.focus.as_deref(), Some("front"));
        assert_eq!(second.effects[0].effect.easing.as_deref(), Some("smooth"));
        let loops = second.loops.as_ref().expect("loop kept as text");
        assert!(loops[0].starts_with("loop {") && loops[0].ends_with("repeats: 3"));
    }

    #[test]
    fn a_number_that_is_not_one_stays_as_written() {
        let doc = document(
            "show \"S\" {\n    @0.0\n    all: static, intensity: 50%, color: \"#FF0000\"\n}\n",
            None,
        )
        .document
        .unwrap();
        let effect = &doc.shows[0].cues[0].effects[0].effect;
        assert_eq!(effect.intensity, None);
        assert_eq!(
            effect.extra.get("intensity").map(String::as_str),
            Some("50%")
        );
        assert_eq!(effect.colors, vec!["#FF0000"]);
    }

    #[test]
    fn a_time_is_whole_milliseconds() {
        for (written, ms) in [
            ("@1.005", 1005.0),
            ("@0.001", 1.0),
            ("@00:00.003", 3.0),
            ("@2:03.5", 123500.0),
            ("@00:01.4999", 1499.0),
        ] {
            let doc = document(&format!("show \"S\" {{\n    {written}\n}}\n"), None)
                .document
                .unwrap();
            assert_eq!(doc.shows[0].cues[0].timestamp.ms, Some(ms), "{written}");
        }
    }

    #[test]
    fn refuses_what_the_editor_does_not_edit() {
        let venue = "venue \"v\" {\n}\nshow \"S\" {\n    @0.0\n}\n";
        assert!(document(venue, None)
            .error
            .unwrap()
            .contains("defines a venue"));
        let own_tempo = "show \"S\" {\n    tempo {\n        bpm: 100\n    }\n    @1/1\n}\n";
        assert!(document(own_tempo, None)
            .error
            .unwrap()
            .contains("tempo block of its own"));
        assert!(document("show \"S\" {\n    @zzz\n}\n", None)
            .error
            .unwrap()
            .contains("parsing error"));
    }
}
