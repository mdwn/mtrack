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
//! its value as written -- so [`super::writer`] can write the file back;
//! where a line lands in time is [`super::layout`]'s business.
//!
//! The types' TypeScript declarations are generated from them (`tsify`)
//! for the web UI, which holds a [`Document`], edits it through
//! [`super::edit`] and never parses or writes the DSL itself.

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use pest::iterators::Pair;
use pest::Parser;
use serde::{Deserialize, Serialize};

use super::super::tempo::{parse_signature_in, SignatureRange};
use super::grammar::{LightingParser, Rule};
use super::include::normalize;
pub use super::types::{EffectKind, LayerCommandType};
use super::utils::{
    parse_measure_time, parse_time_string, parse_time_unit, parse_time_value, unquote, TimeUnit,
    TimeValue,
};

/// A `.light` file as written.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
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
    /// The comment lines after the last item, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_after: Option<Vec<String>>,
    /// Its items in the order they were written (each kind's in its
    /// list's order), so a save keeps them so.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub items: Option<Vec<DocumentItem>>,
}

/// The kinds of item a file holds.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentItem {
    Include,
    Tempo,
    Sequence,
    Show,
}

/// What reading a file as a [`Document`] gives: the document, or why the
/// editor cannot hold the file (it does not parse, or it defines what the
/// editor does not edit).
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Include {
    /// The path as written, relative to the file's folder.
    pub written: String,
    /// The included file's path, resolved against the file's own path
    /// when one is given (else as written): what to fetch it by.
    pub path: String,
    pub line: u32,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
}

/// A `tempo { }` block as written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TempoSection {
    pub start: DurationValue,
    pub bpm: f64,
    /// `bpm:` as written (`120.0`), for a save to keep.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub bpm_text: Option<String>,
    pub time_signature: (u32, u32),
    /// `time_signature:` as written, for a save to keep.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub time_signature_text: Option<String>,
    pub changes: Vec<TempoChange>,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on the `tempo {` line, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// The comments among its lines, each by the field it sits at.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments: Option<Vec<TempoComment>>,
    /// The comment lines after the last change, before the `]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub changes_comments_after: Option<Vec<String>>,
    /// The fields in the order the block wrote them; one it left out is
    /// written after them.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub fields: Option<Vec<TempoField>>,
    /// A comment on its closing brace's line, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub close_comment: Option<String>,
}

/// A comment inside a `tempo { }` block: above the field it names (or on
/// that field's line, `inline`); `end` for one after the last field.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TempoComment {
    pub at: TempoField,
    pub inline: bool,
    pub text: String,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TempoField {
    Start,
    Bpm,
    TimeSignature,
    Changes,
    End,
}

/// One entry of a tempo block's `changes:`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    /// Its parameters as written, in order and spelling (`bpm: 140.0`),
    /// for a save to keep while the values above are still theirs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub written: Option<Vec<Param>>,
    /// Whether the comment on its line sits before its comma.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment_before_comma: Option<bool>,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
}

/// A number with its unit, as a tempo block's `start:` is written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DurationValue {
    pub value: f64,
    pub unit: DurationUnit,
    /// How it was written, for a save to keep while the value and the
    /// unit are still the ones read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub written: Option<WrittenDuration>,
}

/// A duration as written (`0.0s`, `500ms`) and the value and unit it reads
/// as.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WrittenDuration {
    pub text: String,
    pub value: f64,
    pub unit: DurationUnit,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DurationUnit {
    Ms,
    S,
    Beats,
    Measures,
}

impl DurationUnit {
    /// The unit as the grammar writes it after a number.
    pub fn keyword(self) -> &'static str {
        match self {
            DurationUnit::Ms => "ms",
            DurationUnit::S => "s",
            DurationUnit::Beats => "beats",
            DurationUnit::Measures => "measures",
        }
    }
}

/// A cue's or a tempo change's position: a time (`@MM:SS.mmm`, `@S.sss`)
/// in milliseconds, or a bar and beat (`@bar/beat`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Timestamp {
    #[serde(rename = "type")]
    pub kind: TimestampKind,
    /// As written (`@0.000`, `@1/1.5`), for a save to keep; an edit makes
    /// a timestamp without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub text: Option<String>,
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TimestampKind {
    Absolute,
    MeasureBeat,
}

/// A `sequence "name"(params), length: … { }` definition.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its header line, after the `{`, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// The comment lines after its last line, before its closing brace.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_after: Option<Vec<String>>,
    /// A comment on its closing brace's line, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub close_comment: Option<String>,
}

/// A `name: value` in a sequence's parameter list or a reference's
/// arguments; a declared parameter with no default is its name alone.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SequenceArg {
    pub name: String,
    /// As written: a quoted colour, a group, a number, a `$param`…
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub value: Option<String>,
    #[serde(flatten)]
    pub remarks: Remarks,
}

/// A `show "name" { }`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LightShow {
    /// As written; `""` for a show written without a name (the only show
    /// of a file may be, and plays as `default`).
    pub name: String,
    /// Whether it was written with no name at all (`show {`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub unnamed: Option<bool>,
    pub cues: Vec<Cue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub line: Option<u32>,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its header line, after the `{`, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// The comment lines after its last line, before its closing brace.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_after: Option<Vec<String>>,
    /// A comment on its closing brace's line, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub close_comment: Option<String>,
}

/// One `@time` and everything written under it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cue {
    pub timestamp: Timestamp,
    pub effects: Vec<CueEffect>,
    pub commands: Vec<LayerCommand>,
    pub sequences: Vec<SequenceRef>,
    /// What its `offset N measures` commands add up to, when it has any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub offset_measures: Option<u32>,
    /// Each `offset` line as written, with its comments, for a save to
    /// keep the lines while their sum is still the above.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub offsets: Option<Vec<OffsetLine>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub reset_measures: Option<bool>,
    /// Its inline `loop { } repeats: n` blocks, as written: the editor
    /// keeps them as text.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loops: Option<Vec<InlineLoop>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub line: Option<u32>,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its `@` line, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// The comment lines above its `reset_measures` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub reset_comments_before: Option<Vec<String>>,
    /// A comment on its `reset_measures` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub reset_comment: Option<String>,
    /// Its lines in the order they were written (an effect, a command, a
    /// reference, an offset, a reset, a loop; each kind's in its list's
    /// order), so a save keeps them so.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub lines: Option<Vec<CueLine>>,
    /// Which of its lines was read last: where a comment on that line goes.
    #[serde(skip)]
    last: CueLine,
}

/// An `offset N measures` line as written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OffsetLine {
    pub measures: u32,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
}

/// The kinds of line a cue holds.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CueLine {
    #[default]
    Timestamp,
    Effect,
    Command,
    Reference,
    Offset,
    Reset,
    Loop,
}

/// An inline `loop { } repeats: n` block, as written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InlineLoop {
    pub text: String,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
}

/// An effect line: its groups and its parameters.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CueEffect {
    pub groups: Vec<String>,
    /// The comments around each of its groups, one per group, when any
    /// has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub group_remarks: Option<Vec<Remarks>>,
    pub effect: EffectParams,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// Comment lines after it that are its own: those of parameters an
    /// edit removed when none was left to keep them. A file read never
    /// gives one any (comment lines after a line are the next line's).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_after: Option<Vec<String>>,
}

/// An effect's parameters as written: each `name: value` in order, the
/// value as its text (a colour with its quotes, `50%`, `$level`), so a
/// repeated name and a spelling survive a save. The editor's forms read
/// and set them by name ([`super::edit`]).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EffectParams {
    #[serde(rename = "type")]
    pub kind: EffectKind,
    pub params: Vec<Param>,
    /// Whether the line wrote its parameters after the type with no comma
    /// (`static color: …`), as the grammar also allows.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub no_comma: Option<bool>,
}

/// A `name: value` parameter as written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: String,
    /// Whether the value is a sequence parameter reference (`$name`), by
    /// the grammar's `param_ref`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub is_ref: Option<bool>,
    #[serde(flatten)]
    pub remarks: Remarks,
}

/// The comments written around one element of a list -- a parameter, an
/// argument, a group name -- for a save to write back in place.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Remarks {
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it (and its comma), as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
    /// The comment lines after it, when it is the last of its list.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_after: Option<Vec<String>>,
}

impl Remarks {
    fn is_empty(&self) -> bool {
        self == &Remarks::default()
    }
}

/// A `clear(…)`, `freeze(…)`, `unfreeze(…)` or `master(…)`.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LayerCommand {
    pub command: LayerCommandType,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub layer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub intensity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub speed: Option<String>,
    /// Its parameters as written, in order, for a save to keep while the
    /// values above are still theirs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub written: Option<Vec<Param>>,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
}

/// A `sequence "name"(args), loop: …` reference, or a `stop sequence
/// "name"`.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    /// Whether a comma preceded `loop:`, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loop_comma: Option<bool>,
    /// A comment after its arguments' `)` (or its name, and the comma),
    /// on that line, with `loop:` on a later one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loop_comment: Option<String>,
    /// The comment lines between its arguments and its `loop:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loop_comments_before: Option<Vec<String>>,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
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
    let mut pairs = LightingParser::parse(Rule::file, source)
        .map_err(|e| super::error::describe_pest_error(&e, source))?;
    let mut document = Document::default();
    let mut includes = Vec::new();
    let mut comments = Comments::default();
    // What the last item was, for a comment on its last line.
    enum Last {
        None,
        Include,
        Tempo,
        Sequence,
        Show,
    }
    let mut last = Last::None;
    let mut order: Vec<DocumentItem> = Vec::new();
    let file = pairs.next().ok_or("no file")?;
    for item in walk(&file, source) {
        let pair = match item {
            Item::Comment { text, inline, .. } => {
                let slot = match last {
                    Last::Include => includes.last_mut().map(|i: &mut Include| &mut i.comment),
                    Last::Tempo => document.tempo.as_mut().map(|t| &mut t.close_comment),
                    Last::Sequence => document.sequences.last_mut().map(|s| &mut s.close_comment),
                    Last::Show => document.shows.last_mut().map(|s| &mut s.close_comment),
                    Last::None => None,
                };
                comments.keep(text, inline, slot);
                continue;
            }
            Item::Pair(pair) => pair,
        };
        let line = pair.line_col().0 as u32;
        match pair.as_rule() {
            Rule::include => {
                let inner = pair.clone();
                // The file the player reads for it, by the key it reads
                // it by (`Sources::resolve`), or why it reads none.
                let named = super::include::Include::from_pair(pair);
                let path = normalize(&named.target(path)?)
                    .to_string_lossy()
                    .into_owned();
                let mut include = Include {
                    written: named.written,
                    path,
                    line,
                    comments_before: comments.take(),
                    comment: None,
                };
                comments.keep_inner(&inner, source, &mut include.comment);
                includes.push(include);
                last = Last::Include;
                order.push(DocumentItem::Include);
            }
            Rule::tempo => {
                if document.tempo.is_some() {
                    return Err(format!(
                        "line {line}: this file has a second tempo block, which the show editor \
                         cannot hold (the player takes the last); edit the text"
                    )
                    .into());
                }
                let before = comments.take();
                let mut section = tempo_section(pair, source)?;
                section.comments_before = before;
                document.tempo = Some(section);
                last = Last::Tempo;
                order.push(DocumentItem::Tempo);
            }
            Rule::sequence => {
                let mut seq = sequence(pair, source)?;
                seq.comments_before = comments.take();
                document.sequences.push(seq);
                last = Last::Sequence;
                order.push(DocumentItem::Sequence);
            }
            Rule::light_show => {
                let mut show = show(pair, source)?;
                show.comments_before = comments.take();
                document.shows.push(show);
                last = Last::Show;
                order.push(DocumentItem::Show);
            }
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
    document.comments_after = comments.take();
    if !order.is_empty() {
        document.items = Some(order);
    }
    Ok(document)
}

/// What a rule holds, in order: its children, and the comments in the
/// text between them (a gap holds whitespace, punctuation and comments
/// alone; the grammar's `gap` rule reads the comments out of it).
enum Item<'i> {
    /// A comment as written; `inline` when something is written before it
    /// on its line.
    Comment {
        text: String,
        inline: bool,
        /// Whether a comma came before it in the gap.
        after_comma: bool,
        /// Where it starts in the source.
        at: usize,
    },
    Pair(Pair<'i, Rule>),
}

/// `parent`'s children with the comments between them (see [`Item`]):
/// in the text before the first child, between children, and after the
/// last (where the rule's closing `}` or `]` sits).
fn walk<'i>(parent: &Pair<'i, Rule>, source: &'i str) -> Vec<Item<'i>> {
    let mut items = Vec::new();
    let span = parent.as_span();
    let mut at = span.start();
    for child in parent.clone().into_inner() {
        gap_comments(source, at, child.as_span().start(), &mut items);
        at = child.as_span().end();
        items.push(Item::Pair(child));
    }
    gap_comments(source, at, span.end(), &mut items);
    items
}

/// [`walk`] at every depth: a child `part` picks, or one with no children,
/// is an item; any other is entered, the gaps between its own children
/// read too. A value the document keeps as its text (a parameter's value,
/// an inline loop) is an item: its comments are in that text.
fn walk_deep<'i>(
    parent: &Pair<'i, Rule>,
    source: &'i str,
    part: &dyn Fn(Rule) -> bool,
) -> Vec<Item<'i>> {
    let mut items = Vec::new();
    for item in walk(parent, source) {
        match item {
            Item::Pair(child)
                if !part(child.as_rule())
                    && !matches!(child.as_rule(), Rule::parameter_value | Rule::inline_loop)
                    && child.clone().into_inner().next().is_some() =>
            {
                items.extend(walk_deep(&child, source, part))
            }
            item => items.push(item),
        }
    }
    items
}

/// Where the text of `pair` ends: after its last token, before the
/// whitespace and comments a rule that ends in an optional part or a
/// repetition (`effect`, `sequence_reference`) keeps in its span, skipped
/// before the part that did not match. What is after it is on the pair's
/// last line or the lines after it, not inside it.
fn text_end(pair: &Pair<Rule>, source: &str) -> usize {
    // The last pair's end; after it, only punctuation (a `)`), whitespace
    // and comments, which the `gap` rule tells apart.
    fn last(pair: &Pair<Rule>) -> usize {
        match pair.clone().into_inner().last() {
            Some(child) => last(&child),
            None => pair.as_span().end(),
        }
    }
    let from = last(pair);
    let rest = &source[from..pair.as_span().end()];
    let comments = comment_spans(rest);
    let outside = |i: &usize| !comments.iter().any(|(s, e)| (*s..*e).contains(i));
    rest.char_indices()
        .rev()
        .find(|(i, c)| !c.is_whitespace() && outside(i))
        .map(|(i, c)| from + i + c.len_utf8())
        .unwrap_or(from)
}

/// Where the comments of `gap` are, by the grammar's `gap` rule.
fn comment_spans(gap: &str) -> Vec<(usize, usize)> {
    LightingParser::parse(Rule::gap, gap)
        .map(|pairs| {
            pairs
                .flatten()
                .filter(|p| p.as_rule() == Rule::comment_tok)
                .map(|p| (p.as_span().start(), p.as_span().end()))
                .collect()
        })
        .unwrap_or_default()
}

/// `gap`'s text with its comments (by the grammar's `gap` rule) left
/// out: its whitespace and punctuation.
fn outside_comments(gap: &str) -> String {
    let comments = comment_spans(gap);
    gap.char_indices()
        .filter(|(i, _)| !comments.iter().any(|(s, e)| (*s..*e).contains(i)))
        .map(|(_, c)| c)
        .collect()
}

/// The comments in `source[from..to]`, a gap, by the grammar's `gap`
/// rule; one is `inline` when something is written before it on its line.
fn gap_comments(source: &str, from: usize, to: usize, items: &mut Vec<Item>) {
    let gap = &source[from..to];
    let Ok(pairs) = LightingParser::parse(Rule::gap, gap) else {
        return;
    };
    for token in pairs.flatten().filter(|p| p.as_rule() == Rule::comment_tok) {
        let at = from + token.as_span().start();
        let line_start = source[..at].rfind('\n').map_or(0, |i| i + 1);
        items.push(Item::Comment {
            text: token.as_str().trim_end().to_string(),
            inline: !source[line_start..at].trim().is_empty(),
            after_comma: gap[..token.as_span().start()].contains(','),
            at,
        });
    }
}

/// The comments around the elements of a rule -- the pairs `element`
/// picks among `items`, a [`walk_deep`] of it -- one [`Remarks`] per
/// element, in order: the comment lines above an element are its own, as
/// is a comment on its line after it (after its comma, or a `)`), and
/// the comment lines after the last element; a comment inside an element
/// is that element's (on its line, else above it). The comments of a rule
/// with no element are returned apart.
fn remarks(
    items: Vec<Item>,
    source: &str,
    element: &dyn Fn(Rule) -> bool,
) -> (Vec<(Rule, Remarks)>, Vec<String>) {
    let mut out: Vec<(Rule, Remarks)> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    // Whether the last pair was an element: a comment on the line after
    // anything else waits for the next element.
    let mut after_element = false;
    for item in items {
        match item {
            Item::Comment { text, inline, .. } => match out.last_mut() {
                Some((_, last)) if inline && after_element && last.comment.is_none() => {
                    last.comment = Some(text)
                }
                _ => pending.push(text),
            },
            Item::Pair(pair) if element(pair.as_rule()) => {
                let mut own = Remarks {
                    comments_before: take(&mut pending),
                    ..Remarks::default()
                };
                for inner in walk_deep(&pair, source, &|_| false) {
                    if let Item::Comment { text, inline, .. } = inner {
                        if inline && own.comment.is_none() {
                            own.comment = Some(text);
                        } else {
                            own.comments_before.get_or_insert_with(Vec::new).push(text);
                        }
                    }
                }
                out.push((pair.as_rule(), own));
                after_element = true;
            }
            Item::Pair(_) => after_element = false,
        }
    }
    match out.last_mut() {
        Some((_, last)) => {
            if let Some(after) = take(&mut pending) {
                last.comments_after = Some(after);
            }
            (out, Vec::new())
        }
        None => (out, pending),
    }
}

/// The comments inside a pair the document holds as one line or field
/// (`offset 2 measures`, `start: 0s`), wherever they sit in it: each
/// with whether it is on a line after something.
fn inner_comments(pair: &Pair<Rule>, source: &str) -> Vec<(String, bool)> {
    walk_deep(pair, source, &|_| false)
        .into_iter()
        .filter_map(|item| match item {
            Item::Comment { text, inline, .. } => Some((text, inline)),
            Item::Pair(_) => None,
        })
        .collect()
}

/// The comments of a line the document holds element by element (an
/// effect, a command, a reference, a tempo change): each element's
/// [`Remarks`] by its rule, in order (see [`remarks`]); the comments in it
/// with no element to go to; and those after its text (see [`text_end`]),
/// which are on its last line or the lines after it.
struct LineComments<'i> {
    elements: Vec<(Rule, Remarks)>,
    loose: Vec<String>,
    tail: Vec<Item<'i>>,
    after_close: Vec<(String, bool)>,
}

impl LineComments<'_> {
    fn read<'i>(
        pair: &Pair<'i, Rule>,
        source: &'i str,
        element: &dyn Fn(Rule) -> bool,
    ) -> LineComments<'i> {
        Self::read_closed(pair, source, element, None)
    }

    /// [`LineComments::read`] of a line whose elements are a list closed
    /// at `close` (after its `)`) with more of the line after it: the
    /// comments after the close are the line's (`after_close`, each with
    /// whether it is on a line after something), not the last element's.
    fn read_closed<'i>(
        pair: &Pair<'i, Rule>,
        source: &'i str,
        element: &dyn Fn(Rule) -> bool,
        close: Option<usize>,
    ) -> LineComments<'i> {
        let end = text_end(pair, source);
        let close = close.unwrap_or(end);
        let mut tail = Vec::new();
        let mut body = Vec::new();
        let mut after_close = Vec::new();
        for item in walk_deep(pair, source, element) {
            match item {
                Item::Comment { at, .. } if at >= end => tail.push(item),
                Item::Comment {
                    at, text, inline, ..
                } if at >= close => after_close.push((text, inline)),
                item => body.push(item),
            }
        }
        let (elements, loose) = remarks(body, source, element);
        LineComments {
            elements,
            loose,
            tail,
            after_close,
        }
    }

    /// The remarks of the elements of `rule`, in order.
    fn of(&mut self, rule: Rule) -> impl Iterator<Item = Remarks> + '_ {
        self.elements
            .iter_mut()
            .filter(move |(r, _)| *r == rule)
            .map(|(_, remarks)| std::mem::take(remarks))
    }
}

/// Comment lines `before` the ones `after`, when there are any.
fn joined(before: Option<Vec<String>>, after: Option<Vec<String>>) -> Option<Vec<String>> {
    match (before, after) {
        (Some(mut before), Some(after)) => {
            before.extend(after);
            Some(before)
        }
        (before, after) => before.or(after),
    }
}

/// `pending`, emptied, when it holds anything.
fn take(pending: &mut Vec<String>) -> Option<Vec<String>> {
    if pending.is_empty() {
        None
    } else {
        Some(std::mem::take(pending))
    }
}

/// The comments of a rule as they are read: one on a line's own goes in
/// that line's slot (a line that holds one already, or no line at all,
/// keeps it as a line of its own); one on a line of its own waits
/// (`pending`) for the next line, or for the end of the block it is in.
#[derive(Default)]
struct Comments {
    pending: Vec<String>,
}

impl Comments {
    fn keep(&mut self, text: String, inline: bool, slot: Option<&mut Option<String>>) {
        match (inline, slot) {
            (true, Some(slot @ None)) => *slot = Some(text),
            _ => self.pending.push(text),
        }
    }

    /// The comment lines waiting, for the line that comes next.
    fn take(&mut self) -> Option<Vec<String>> {
        take(&mut self.pending)
    }

    /// The comments inside a line or a field (see [`inner_comments`]):
    /// one on its line goes in its slot, the others wait for the next.
    fn keep_inner(&mut self, pair: &Pair<Rule>, source: &str, slot: &mut Option<String>) {
        for (text, inline) in inner_comments(pair, source) {
            match slot {
                None if inline => *slot = Some(text),
                _ => self.pending.push(text),
            }
        }
    }
}

fn tempo_section(pair: Pair<Rule>, source: &str) -> Result<TempoSection, Box<dyn Error>> {
    let mut section = TempoSection {
        start: DurationValue {
            value: 0.0,
            unit: DurationUnit::S,
            written: None,
        },
        bpm: crate::tempo::DEFAULT_BPM,
        bpm_text: None,
        time_signature: (
            crate::tempo::DEFAULT_TIME_SIGNATURE.numerator,
            crate::tempo::DEFAULT_TIME_SIGNATURE.denominator,
        ),
        time_signature_text: None,
        changes: Vec::new(),
        comments_before: None,
        comment: None,
        comments: None,
        changes_comments_after: None,
        fields: None,
        close_comment: None,
    };
    let mut comments = Comments::default();
    let mut inner: Vec<TempoComment> = Vec::new();
    let mut fields: Vec<TempoField> = Vec::new();
    // The field whose line a comment on the same line belongs to.
    let mut last: Option<TempoField> = None;
    let place = |pending: Option<Vec<String>>, at: TempoField, inner: &mut Vec<TempoComment>| {
        for text in pending.into_iter().flatten() {
            inner.push(TempoComment {
                at,
                inline: false,
                text,
            });
        }
    };
    let keep = |text: String,
                inline: bool,
                last: Option<TempoField>,
                inner: &mut Vec<TempoComment>,
                comments: &mut Comments,
                header: &mut Option<String>| {
        match (inline, last) {
            (true, Some(at)) => inner.push(TempoComment {
                at,
                inline: true,
                text,
            }),
            (true, None) => comments.keep(text, true, Some(header)),
            (false, _) => comments.keep(text, false, None),
        }
    };
    for item in walk(&pair, source) {
        let content = match item {
            Item::Comment { text, inline, .. } => {
                keep(
                    text,
                    inline,
                    last,
                    &mut inner,
                    &mut comments,
                    &mut section.comment,
                );
                continue;
            }
            Item::Pair(pair) => pair,
        };
        if content.as_rule() != Rule::tempo_content {
            continue;
        }
        for item in walk(&content, source) {
            let item = match item {
                Item::Comment { text, inline, .. } => {
                    keep(
                        text,
                        inline,
                        last,
                        &mut inner,
                        &mut comments,
                        &mut section.comment,
                    );
                    continue;
                }
                Item::Pair(pair) => pair,
            };
            match item.as_rule() {
                Rule::tempo_start => {
                    place(comments.take(), TempoField::Start, &mut inner);
                    fields.push(TempoField::Start);
                    last = Some(TempoField::Start);
                    for (text, inline) in inner_comments(&item, source) {
                        keep(
                            text,
                            inline,
                            last,
                            &mut inner,
                            &mut comments,
                            &mut section.comment,
                        );
                    }
                    if let Some(value) = item
                        .into_inner()
                        .find(|p| p.as_rule() == Rule::time_parameter)
                    {
                        let raw = value.as_str().trim();
                        if !raw.is_empty() {
                            // Refused as the player refuses it.
                            super::tempo_parse::parse_tempo_start(raw)?;
                            section.start = duration_value(raw)?;
                        }
                    }
                }
                Rule::tempo_bpm => {
                    place(comments.take(), TempoField::Bpm, &mut inner);
                    fields.push(TempoField::Bpm);
                    last = Some(TempoField::Bpm);
                    for (text, inline) in inner_comments(&item, source) {
                        keep(
                            text,
                            inline,
                            last,
                            &mut inner,
                            &mut comments,
                            &mut section.comment,
                        );
                    }
                    if let Some(value) = item
                        .into_inner()
                        .find(|p| p.as_rule() == Rule::number_value)
                    {
                        section.bpm = value.as_str().trim().parse()?;
                        section.bpm_text = Some(value.as_str().trim().to_string());
                    }
                }
                Rule::tempo_time_signature => {
                    place(comments.take(), TempoField::TimeSignature, &mut inner);
                    fields.push(TempoField::TimeSignature);
                    last = Some(TempoField::TimeSignature);
                    for (text, inline) in inner_comments(&item, source) {
                        keep(
                            text,
                            inline,
                            last,
                            &mut inner,
                            &mut comments,
                            &mut section.comment,
                        );
                    }
                    if let Some(value) = item
                        .into_inner()
                        .find(|p| p.as_rule() == Rule::time_sig_value)
                    {
                        section.time_signature = signature(value.as_str())?;
                        section.time_signature_text = Some(value.as_str().trim().to_string());
                    }
                }
                Rule::tempo_changes => {
                    place(comments.take(), TempoField::Changes, &mut inner);
                    fields.push(TempoField::Changes);
                    tempo_changes(item, source, &mut section, &mut comments)?;
                    last = Some(TempoField::End);
                }
                _ => {}
            }
        }
    }
    place(comments.take(), TempoField::End, &mut inner);
    if !inner.is_empty() {
        section.comments = Some(inner);
    }
    if !fields.is_empty() {
        section.fields = Some(fields);
    }
    Ok(section)
}

/// The `changes: [ … ]` of a tempo block, with the comments among them.
fn tempo_changes(
    pair: Pair<Rule>,
    source: &str,
    section: &mut TempoSection,
    comments: &mut Comments,
) -> Result<(), Box<dyn Error>> {
    // A comment on a change's line, before or after its comma.
    let keep = |text: String,
                inline: bool,
                after_comma: bool,
                changes: &mut Vec<TempoChange>,
                comments: &mut Comments| {
        if inline && !after_comma {
            if let Some(change) = changes.last_mut().filter(|c| c.comment.is_none()) {
                change.comment_before_comma = Some(true);
            }
        }
        let slot = changes.last_mut().map(|c| &mut c.comment);
        comments.keep(text, inline, slot);
    };
    for item in walk(&pair, source) {
        let list = match item {
            Item::Comment {
                text,
                inline,
                after_comma,
                ..
            } => {
                keep(text, inline, after_comma, &mut section.changes, comments);
                continue;
            }
            Item::Pair(pair) => pair,
        };
        if list.as_rule() != Rule::tempo_change_list {
            continue;
        }
        for item in walk(&list, source) {
            match item {
                Item::Comment {
                    text,
                    inline,
                    after_comma,
                    ..
                } => {
                    keep(text, inline, after_comma, &mut section.changes, comments);
                }
                Item::Pair(change) if change.as_rule() == Rule::tempo_change => {
                    let mut change = tempo_change(change, source)?;
                    change.comments_before = comments.take();
                    section.changes.push(change);
                }
                Item::Pair(_) => {}
            }
        }
    }
    // The last change has no comma for a comment to sit before: one on
    // its line follows it, and one written after it follows its comma
    // when a change is added after it.
    if let Some(last) = section.changes.last_mut() {
        last.comment_before_comma = None;
    }
    section.changes_comments_after = comments.take();
    Ok(())
}

fn tempo_change(pair: Pair<Rule>, source: &str) -> Result<TempoChange, Box<dyn Error>> {
    let mut found = LineComments::read(&pair, source, &|r| r == Rule::tempo_change_param);
    let mut change = TempoChange {
        timestamp: Timestamp {
            kind: TimestampKind::Absolute,
            text: None,
            ms: Some(0.0),
            measure: None,
            beat: None,
        },
        bpm: None,
        time_signature: None,
        transition: None,
        written: None,
        comment_before_comma: None,
        comments_before: None,
        comment: None,
    };
    let mut written = Vec::new();
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
                    let text = value.as_str().trim().to_string();
                    let name = match rule {
                        Rule::tempo_bpm => {
                            change.bpm = Some(text.parse()?);
                            "bpm"
                        }
                        Rule::tempo_time_signature => {
                            change.time_signature = Some(signature(&text)?);
                            "time_signature"
                        }
                        Rule::tempo_change_transition => {
                            change.transition = Some(text.clone());
                            "transition"
                        }
                        _ => continue,
                    };
                    written.push(Param {
                        name: name.to_string(),
                        value: text,
                        is_ref: None,
                        remarks: Remarks::default(),
                    });
                }
            }
            _ => {}
        }
    }
    for (param, remarks) in written.iter_mut().zip(found.of(Rule::tempo_change_param)) {
        param.remarks = remarks;
    }
    if !written.is_empty() {
        change.written = Some(written);
    }
    Ok(change)
}

/// `4/4` as `(4, 4)`, as written: the player's own range check is in
/// `parse_signature_in`, applied when the file loads.
/// A `time_signature:` as the player reads it (`parse_signature_in`, a
/// light show's range).
fn signature(raw: &str) -> Result<(u32, u32), Box<dyn Error>> {
    let sig = parse_signature_in(raw, SignatureRange::Light)?;
    Ok((sig.numerator, sig.denominator))
}

/// `2.5s`, `500ms`, `1beat`, `2measures` as the number and its unit, by
/// the player's one table (`parse_time_unit`).
fn duration_value(raw: &str) -> Result<DurationValue, Box<dyn Error>> {
    let (value, unit) = parse_time_unit(raw)?;
    let unit = match unit {
        TimeUnit::Ms => DurationUnit::Ms,
        TimeUnit::S => DurationUnit::S,
        TimeUnit::Beats => DurationUnit::Beats,
        TimeUnit::Measures => DurationUnit::Measures,
    };
    Ok(DurationValue {
        value,
        unit,
        written: Some(WrittenDuration {
            text: raw.trim().to_string(),
            value,
            unit,
        }),
    })
}

/// The time `value` written in `from` (`s` or `ms`, as a tempo block's
/// `start:` is) written in `to` instead: the shortest decimal the player
/// reads (`parse_time_value`) as the same nanoseconds, so switching the
/// unit never moves the time. `None` for a unit or a value that is not
/// a time.
pub fn time_in(value: f64, from: &str, to: &str) -> Option<f64> {
    let read = |v: f64, unit: &str| match parse_time_value(&format!("{v}{unit}")).ok()? {
        TimeValue::Seconds(secs) => Duration::try_from_secs_f64(secs).ok(),
        _ => None,
    };
    let time = read(value, from)?;
    let raw = match to {
        "s" => time.as_secs_f64(),
        "ms" => time.as_secs_f64() * 1000.0,
        _ => return None,
    };
    (0..=17)
        .filter_map(|places| format!("{raw:.places$}").parse::<f64>().ok())
        .chain([raw])
        .find(|&v| read(v, to) == Some(time))
}

/// A tempo change's `transition:` value (`snap`, `4`, `2m`) as the
/// player reads it (`tempo_parse::parse_transition`); `None` for text the
/// grammar does not read as one, whole.
pub fn transition(text: &str) -> Option<super::super::tempo::TempoTransition> {
    let text = text.trim();
    let pair = LightingParser::parse(Rule::tempo_transition_duration, text)
        .ok()?
        .next()?;
    if pair.as_span().end() != text.len() {
        return None;
    }
    super::tempo_parse::parse_transition(pair).ok()
}

fn timestamp(pair: Pair<Rule>) -> Result<Timestamp, Box<dyn Error>> {
    Ok(match pair.as_rule() {
        Rule::measure_time => {
            let (measure, beat) = parse_measure_time(pair.as_str())?;
            Timestamp {
                kind: TimestampKind::MeasureBeat,
                text: Some(pair.as_str().trim().to_string()),
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
            text: Some(pair.as_str().trim().to_string()),
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
        comments_before: None,
        comment: None,
        comments_after: None,
        close_comment: None,
    };
    let mut comments = Comments::default();
    for item in walk(&pair, source) {
        let item = match item {
            Item::Comment { text, inline, .. } => {
                let slot = match sequence.cues.last_mut() {
                    Some(cue) => cue.last_line(),
                    None => &mut sequence.comment,
                };
                comments.keep(text, inline, Some(slot));
                continue;
            }
            Item::Pair(pair) => pair,
        };
        match item.as_rule() {
            Rule::sequence_name => sequence.name = unquote(&item),
            Rule::sequence_param_decls => {
                let mut found =
                    LineComments::read(&item, source, &|r| r == Rule::sequence_param_decl);
                let mut params: Vec<SequenceArg> = item
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::sequence_param_decl)
                    .map(arg)
                    .collect();
                for (param, remarks) in params.iter_mut().zip(found.of(Rule::sequence_param_decl)) {
                    param.remarks = remarks;
                }
                comments.pending.extend(found.loose);
                sequence.params = Some(params);
            }
            Rule::sequence_length => {
                comments.keep_inner(&item, source, &mut sequence.comment);
                sequence.length = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::time_parameter)
                    .map(|p| p.as_str().trim().to_string())
            }
            Rule::sequence_content => {
                sequence.cues = cues(
                    item,
                    source,
                    "a sequence",
                    &mut comments,
                    &mut sequence.comment,
                )?;
            }
            _ => {}
        }
    }
    sequence.comments_after = comments.take();
    Ok(sequence)
}

fn show(pair: Pair<Rule>, source: &str) -> Result<LightShow, Box<dyn Error>> {
    let line = pair.line_col().0 as u32;
    let mut show = LightShow {
        name: String::new(),
        unnamed: Some(true),
        cues: Vec::new(),
        line: Some(line),
        comments_before: None,
        comment: None,
        comments_after: None,
        close_comment: None,
    };
    let mut comments = Comments::default();
    for item in walk(&pair, source) {
        let item = match item {
            Item::Comment { text, inline, .. } => {
                let slot = match show.cues.last_mut() {
                    Some(cue) => cue.last_line(),
                    None => &mut show.comment,
                };
                comments.keep(text, inline, Some(slot));
                continue;
            }
            Item::Pair(pair) => pair,
        };
        match item.as_rule() {
            Rule::show_name => {
                show.name = unquote(&item);
                show.unnamed = None;
            }
            Rule::show_content => {
                show.cues = cues(item, source, "a show", &mut comments, &mut show.comment)?;
            }
            _ => {}
        }
    }
    show.comments_after = comments.take();
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
        remarks: Remarks::default(),
    }
}

/// The cues of a show's or a sequence's body, each with the comment lines
/// above it; a comment on a line of the body's is that line's, one on the
/// `{` line the header's.
fn cues(
    content: Pair<Rule>,
    source: &str,
    what: &str,
    comments: &mut Comments,
    header: &mut Option<String>,
) -> Result<Vec<Cue>, Box<dyn Error>> {
    let mut cues: Vec<Cue> = Vec::new();
    for item in walk(&content, source) {
        let item = match item {
            Item::Comment { text, inline, .. } => {
                let slot = match cues.last_mut() {
                    Some(cue) => cue.last_line(),
                    None => header,
                };
                comments.keep(text, inline, Some(slot));
                continue;
            }
            Item::Pair(pair) => pair,
        };
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
                let (mut cue, trailing) = cue(item, source)?;
                cue.comments_before = comments.take();
                cues.push(cue);
                comments.pending.extend(trailing.into_iter().flatten());
            }
            _ => {}
        }
    }
    Ok(cues)
}

impl Cue {
    /// The slot a comment on the cue's last line goes in.
    fn last_line(&mut self) -> &mut Option<String> {
        match self.last {
            CueLine::Timestamp => &mut self.comment,
            CueLine::Effect => &mut self.effects.last_mut().expect("an effect").comment,
            CueLine::Command => &mut self.commands.last_mut().expect("a command").comment,
            CueLine::Reference => &mut self.sequences.last_mut().expect("a reference").comment,
            CueLine::Offset => {
                &mut self
                    .offsets
                    .as_mut()
                    .and_then(|o| o.last_mut())
                    .expect("an offset")
                    .comment
            }
            CueLine::Reset => &mut self.reset_comment,
            CueLine::Loop => {
                &mut self
                    .loops
                    .as_mut()
                    .and_then(|l| l.last_mut())
                    .expect("a loop")
                    .comment
            }
        }
    }
}

/// A cue as read, with the comment lines its span holds after its last
/// line (a repetition's skipped tail stays in the span), which belong to
/// what follows the cue.
type CueAndTrailing = (Cue, Option<Vec<String>>);

fn cue(pair: Pair<Rule>, source: &str) -> Result<CueAndTrailing, Box<dyn Error>> {
    let line = pair.line_col().0 as u32;
    let mut cue = Cue {
        timestamp: Timestamp {
            kind: TimestampKind::Absolute,
            text: None,
            ms: Some(0.0),
            measure: None,
            beat: None,
        },
        effects: Vec::new(),
        commands: Vec::new(),
        sequences: Vec::new(),
        comment: None,
        offset_measures: None,
        offsets: None,
        reset_measures: None,
        loops: None,
        line: Some(line),
        comments_before: None,
        reset_comments_before: None,
        reset_comment: None,
        lines: None,
        last: CueLine::Timestamp,
    };
    let mut comments = Comments::default();
    let mut lines: Vec<CueLine> = Vec::new();
    for item in walk(&pair, source) {
        let item = match item {
            Item::Comment { text, inline, .. } => {
                let slot = cue.last_line();
                comments.keep(text, inline, Some(slot));
                continue;
            }
            Item::Pair(pair) => pair,
        };
        match item.as_rule() {
            Rule::time_string | Rule::measure_time => cue.timestamp = timestamp(item)?,
            Rule::effect => {
                let (mut effect, tail) = effect(item, source)?;
                effect.comments_before = joined(comments.take(), effect.comments_before);
                cue.effects.push(effect);
                cue.last = CueLine::Effect;
                lines.push(CueLine::Effect);
                cue.keep_tail(tail, &mut comments);
            }
            Rule::layer_command => {
                let (mut command, tail) = layer_command(item, source)?;
                command.comments_before = joined(comments.take(), command.comments_before);
                cue.commands.push(command);
                cue.last = CueLine::Command;
                lines.push(CueLine::Command);
                cue.keep_tail(tail, &mut comments);
            }
            Rule::sequence_reference => {
                let (mut reference, tail) = sequence_ref(item, source);
                reference.comments_before = joined(comments.take(), reference.comments_before);
                cue.sequences.push(reference);
                cue.last = CueLine::Reference;
                lines.push(CueLine::Reference);
                cue.keep_tail(tail, &mut comments);
            }
            Rule::stop_sequence_command => {
                let inner = item.clone();
                let name = item
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::sequence_name)
                    .map(|p| unquote(&p))
                    .unwrap_or_default();
                cue.sequences.push(SequenceRef {
                    name,
                    args: None,
                    loop_: None,
                    stop: Some(true),
                    loop_comma: None,
                    loop_comment: None,
                    loop_comments_before: None,
                    comments_before: comments.take(),
                    comment: None,
                });
                cue.last = CueLine::Reference;
                lines.push(CueLine::Reference);
                comments.keep_inner(&inner, source, cue.last_line());
            }
            Rule::offset_command => {
                let inner = item.clone();
                let measures = super::show::parse_offset_command(item)?;
                cue.offset_measures = Some(cue.offset_measures.unwrap_or(0) + measures);
                cue.offsets.get_or_insert_with(Vec::new).push(OffsetLine {
                    measures,
                    comments_before: comments.take(),
                    comment: None,
                });
                cue.last = CueLine::Offset;
                lines.push(CueLine::Offset);
                comments.keep_inner(&inner, source, cue.last_line());
            }
            Rule::reset_measures_command => {
                cue.reset_measures = Some(true);
                if let Some(pending) = comments.take() {
                    cue.reset_comments_before
                        .get_or_insert_with(Vec::new)
                        .extend(pending);
                }
                cue.last = CueLine::Reset;
                lines.push(CueLine::Reset);
                comments.keep_inner(&item, source, cue.last_line());
            }
            Rule::inline_loop => {
                cue.loops.get_or_insert_with(Vec::new).push(InlineLoop {
                    text: item.as_str().to_string(),
                    comments_before: comments.take(),
                    comment: None,
                });
                cue.last = CueLine::Loop;
                lines.push(CueLine::Loop);
            }
            _ => {}
        }
    }
    if !lines.is_empty() {
        cue.lines = Some(lines);
    }
    Ok((cue, comments.take()))
}

impl Cue {
    /// The comments after a line's text (see [`LineComments`]): one on
    /// its line is the line's, the lines after it wait for what follows.
    fn keep_tail(&mut self, tail: Vec<Item>, comments: &mut Comments) {
        for found in tail {
            if let Item::Comment { text, inline, .. } = found {
                comments.keep(text, inline, Some(self.last_line()));
            }
        }
    }
}

/// An effect line, with the comments after its text (see [`LineComments`]).
fn effect<'i>(
    pair: Pair<'i, Rule>,
    whole: &'i str,
) -> Result<(CueEffect, Vec<Item<'i>>), Box<dyn Error>> {
    let mut found = LineComments::read(&pair, whole, &|r| {
        matches!(r, Rule::group_name | Rule::parameter)
    });
    let source = pair.as_str();
    let base = pair.as_span().start();
    let mut groups = Vec::new();
    let mut params = EffectParams {
        kind: EffectKind::Static,
        params: Vec::new(),
        no_comma: None,
    };
    let mut type_end = None;
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::group_list => {
                groups.extend(item.into_inner().map(|g| g.as_str().trim().to_string()))
            }
            Rule::effect_type => {
                type_end = Some(item.as_span().end() - base);
                params.kind = EffectKind::from_keyword(item.as_str())
                    .ok_or_else(|| format!("Unknown effect type: {}", item.as_str()))?;
            }
            Rule::parameters => {
                if let Some(end) = type_end {
                    let between = &source[end..item.as_span().start() - base];
                    if !between.contains(',') {
                        params.no_comma = Some(true);
                    }
                }
                for parameter in item.into_inner().filter(|p| p.as_rule() == Rule::parameter) {
                    let mut parts = parameter.into_inner();
                    let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let is_ref = value
                        .clone()
                        .into_inner()
                        .next()
                        .is_some_and(|v| v.as_rule() == Rule::param_ref);
                    params.params.push(Param {
                        name: name.as_str().trim().to_string(),
                        value: value.as_str().trim().to_string(),
                        is_ref: is_ref.then_some(true),
                        remarks: Remarks::default(),
                    });
                }
            }
            _ => {}
        }
    }
    for (param, remarks) in params.params.iter_mut().zip(found.of(Rule::parameter)) {
        param.remarks = remarks;
    }
    let group_remarks: Vec<Remarks> = found.of(Rule::group_name).collect();
    Ok((
        CueEffect {
            groups,
            group_remarks: group_remarks
                .iter()
                .any(|r| !r.is_empty())
                .then_some(group_remarks),
            effect: params,
            comments_before: take(&mut found.loose),
            comment: None,
            comments_after: None,
        },
        found.tail,
    ))
}

/// A layer command, with the comments after its text (see
/// [`LineComments`]).
fn layer_command<'i>(
    pair: Pair<'i, Rule>,
    source: &'i str,
) -> Result<(LayerCommand, Vec<Item<'i>>), Box<dyn Error>> {
    let mut found = LineComments::read(&pair, source, &|r| r == Rule::layer_command_param);
    let mut command = LayerCommand {
        command: LayerCommandType::Clear,
        layer: None,
        intensity: None,
        speed: None,
        written: None,
        comments_before: None,
        comment: None,
    };
    let mut written = Vec::new();
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::layer_command_type => {
                command.command = LayerCommandType::from_keyword(item.as_str())?;
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
                    let text = value.as_str().trim().to_string();
                    written.push(Param {
                        name: name.as_str().trim().to_string(),
                        value: text.clone(),
                        is_ref: None,
                        remarks: Remarks::default(),
                    });
                    let value = Some(text);
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
    for (param, remarks) in written.iter_mut().zip(found.of(Rule::layer_command_param)) {
        param.remarks = remarks;
    }
    if !written.is_empty() {
        command.written = Some(written);
    }
    command.comments_before = take(&mut found.loose);
    Ok((command, found.tail))
}

/// A sequence reference, with the comments after its text (see
/// [`LineComments`]).
fn sequence_ref<'i>(pair: Pair<'i, Rule>, whole: &'i str) -> (SequenceRef, Vec<Item<'i>>) {
    // Its arguments close at their `)`, or its name ends the list.
    let close = pair
        .clone()
        .into_inner()
        .filter(|p| matches!(p.as_rule(), Rule::sequence_name | Rule::sequence_args))
        .last()
        .map(|p| p.as_span().end());
    let mut found = LineComments::read_closed(&pair, whole, &|r| r == Rule::sequence_arg, close);
    let mut reference = SequenceRef {
        name: String::new(),
        args: None,
        loop_: None,
        stop: None,
        loop_comma: None,
        loop_comment: None,
        loop_comments_before: None,
        comments_before: None,
        comment: None,
    };
    for (text, inline) in std::mem::take(&mut found.after_close) {
        match &mut reference.loop_comment {
            slot @ None if inline => *slot = Some(text),
            _ => reference
                .loop_comments_before
                .get_or_insert_with(Vec::new)
                .push(text),
        }
    }
    let source = pair.as_str();
    let base = pair.as_span().start();
    let mut before_params = 0;
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::sequence_name => {
                reference.name = unquote(&item);
                before_params = item.as_span().end() - base;
            }
            Rule::sequence_args => {
                before_params = item.as_span().end() - base;
                reference.args = Some(
                    item.into_inner()
                        .filter(|p| p.as_rule() == Rule::sequence_arg)
                        .map(arg)
                        .collect(),
                )
            }
            Rule::sequence_params => {
                if outside_comments(&source[before_params..item.as_span().start() - base])
                    .contains(',')
                {
                    reference.loop_comma = Some(true);
                }
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
    if let Some(args) = reference.args.as_mut() {
        for (arg, remarks) in args.iter_mut().zip(found.of(Rule::sequence_arg)) {
            arg.remarks = remarks;
        }
    }
    reference.comments_before = take(&mut found.loose);
    (reference, found.tail)
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
        assert_eq!(seq.cues[0].effects[0].effect.params[0].value, "$hit");
        let show = &doc.shows[0];
        assert_eq!(show.name, "Song");
        let first = &show.cues[0];
        assert_eq!(
            first.comments_before.as_deref(),
            Some(
                &[
                    "# The opening look".to_string(),
                    "# held for a bar".to_string()
                ][..]
            )
        );
        assert_eq!(first.timestamp.kind, TimestampKind::MeasureBeat);
        let effect = &first.effects[0];
        assert_eq!(effect.groups, vec!["front", "back"]);
        assert_eq!(effect.effect.kind, EffectKind::Static);
        let written: Vec<(&str, &str)> = effect
            .effect
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.value.as_str()))
            .collect();
        assert_eq!(
            written,
            [
                ("color", "\"blue\""),
                ("intensity", "0.6"),
                ("dimmer", "80%"),
                ("duration", "1measure"),
                ("layer", "foreground"),
            ]
        );
        assert_eq!(effect.effect.no_comma, None);
        assert_eq!(first.commands[0].command, LayerCommandType::Clear);
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
        assert_eq!(
            second.comments_before.as_deref(),
            Some(&["// a stop".to_string()][..])
        );
        assert_eq!(second.timestamp.beat, Some(1.5));
        assert_eq!(second.sequences[0].stop, Some(true));
        assert_eq!(second.reset_measures, Some(true));
        assert_eq!(second.effects[0].effect.params[0].value, "\"front\"");
        assert_eq!(second.effects[0].effect.params[1].name, "easing");
        let loops = second.loops.as_ref().expect("loop kept as text");
        assert!(loops[0].text.starts_with("loop {") && loops[0].text.ends_with("repeats: 3"));
    }

    #[test]
    fn parameters_are_kept_as_written_repeats_and_all() {
        let doc = document(
            "show \"S\" {\n    @0.0\n    all: static intensity: 50%, intensity: 0.5, color: \"#FF0000\"\n}\n",
            None,
        )
        .document
        .unwrap();
        let effect = &doc.shows[0].cues[0].effects[0].effect;
        let written: Vec<(&str, &str)> = effect
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.value.as_str()))
            .collect();
        assert_eq!(
            written,
            [
                ("intensity", "50%"),
                ("intensity", "0.5"),
                ("color", "\"#FF0000\"")
            ]
        );
        assert_eq!(effect.no_comma, Some(true));
    }

    #[test]
    fn every_comment_is_kept_where_it_was() {
        let source = "# top\ninclude \"a.light\" # inc\n// before tempo\ntempo { # head\n    # before start\n    start: 0s // on start\n    bpm: 120\n    # before changes\n    changes: [\n        # before change\n        @3/1 { bpm: 90 }, # on change\n        @4/1 { bpm: 100 }\n        # after changes\n    ]\n    # at end\n} # close\nsequence \"s\" { # seq head\n    @1/1\n    a: static, color: \"red\", duration: 1s\n    # seq after\n}\n# before show\nshow \"S\" { // show head\n    # before cue\n    @1/1 # on cue\n    # before effect\n    front: static, color: \"red\", duration: 1s # on effect\n    clear(layer: background) # on command\n    # before ref\n    sequence \"s\" # on ref\n    offset 1 measures # on offset\n    reset_measures # on reset\n    # between cues\n    @2/1\n    loop {\n        @0.0\n        a: strobe, frequency: 4, duration: 0.5s\n    } repeats: 2 # on loop\n    # show after\n} # show close\n# file end\n";
        let doc = document(source, Some(Path::new("show.light")))
            .document
            .expect("parses");
        let v = |xs: &[&str]| Some(xs.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        let inc = &doc.includes.as_ref().unwrap()[0];
        assert_eq!(inc.comments_before, v(&["# top"]));
        assert_eq!(inc.comment.as_deref(), Some("# inc"));
        let tempo = doc.tempo.as_ref().unwrap();
        assert_eq!(tempo.comments_before, v(&["// before tempo"]));
        assert_eq!(tempo.comment.as_deref(), Some("# head"));
        assert_eq!(tempo.close_comment.as_deref(), Some("# close"));
        let inner = tempo.comments.as_ref().unwrap();
        let at = |field: TempoField, inline: bool| {
            inner
                .iter()
                .filter(|c| c.at == field && c.inline == inline)
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(at(TempoField::Start, false), ["# before start"]);
        assert_eq!(at(TempoField::Start, true), ["// on start"]);
        assert_eq!(at(TempoField::Changes, false), ["# before changes"]);
        assert_eq!(at(TempoField::End, false), ["# at end"]);
        assert_eq!(tempo.changes[0].comments_before, v(&["# before change"]));
        assert_eq!(tempo.changes[0].comment.as_deref(), Some("# on change"));
        assert_eq!(tempo.changes_comments_after, v(&["# after changes"]));
        let seq = &doc.sequences[0];
        assert_eq!(seq.comment.as_deref(), Some("# seq head"));
        assert_eq!(seq.comments_after, v(&["# seq after"]));
        let show = &doc.shows[0];
        assert_eq!(show.comments_before, v(&["# before show"]));
        assert_eq!(show.comment.as_deref(), Some("// show head"));
        assert_eq!(show.close_comment.as_deref(), Some("# show close"));
        assert_eq!(show.comments_after, v(&["# show after"]));
        let cue = &show.cues[0];
        assert_eq!(cue.comments_before, v(&["# before cue"]));
        assert_eq!(cue.comment.as_deref(), Some("# on cue"));
        assert_eq!(cue.effects[0].comments_before, v(&["# before effect"]));
        assert_eq!(cue.effects[0].comment.as_deref(), Some("# on effect"));
        assert_eq!(cue.commands[0].comment.as_deref(), Some("# on command"));
        assert_eq!(cue.sequences[0].comments_before, v(&["# before ref"]));
        assert_eq!(cue.sequences[0].comment.as_deref(), Some("# on ref"));
        assert_eq!(
            cue.offsets.as_ref().unwrap()[0].comment.as_deref(),
            Some("# on offset")
        );
        assert_eq!(cue.reset_comment.as_deref(), Some("# on reset"));
        let second = &show.cues[1];
        assert_eq!(second.comments_before, v(&["# between cues"]));
        assert_eq!(
            second.loops.as_ref().unwrap()[0].comment.as_deref(),
            Some("# on loop")
        );
        assert_eq!(doc.comments_after, v(&["# file end"]));
    }

    /// A file with a comment between the elements of every kind of list:
    /// a tempo change's parameters, a sequence's declared parameters, a
    /// reference's arguments, a group list, an effect's parameters and a
    /// layer command's.
    const LIST_COMMENTS: &str = "tempo {\n    bpm: 100\n    changes: [\n        @2/1 { bpm: 120, # c1\n            transition: 2m }\n    ]\n}\nsequence \"x\"(\n    hit, # c2\n    bed // d2\n), length: 1measure {\n    @1/1\n    a: static, color: $hit, duration: 1beat\n}\nshow \"S\" {\n    @1/1\n    movers: move, pan: 45deg, # c3\n        tilt: 10deg, duration: 1s\n    master(layer: background, # c4\n        intensity: 50%)\n    sequence \"x\"(hit: \"#FF0000\", # c5\n        bed: \"#0000FF\")\n    front, # c6\n    back: static, color: \"red\", duration: 1s\n}\n";

    #[test]
    fn a_comment_between_the_elements_of_a_list_is_the_elements() {
        let doc = document(LIST_COMMENTS, None).document.expect("parses");
        let c = |text: &str| Some(text.to_string());
        let change = &doc.tempo.as_ref().unwrap().changes[0];
        let written = change.written.as_ref().unwrap();
        assert_eq!(written[0].remarks.comment, c("# c1"));
        assert_eq!(written[1].remarks, Remarks::default());
        let params = doc.sequences[0].params.as_ref().unwrap();
        assert_eq!(params[0].remarks.comment, c("# c2"));
        assert_eq!(params[1].remarks.comment, c("// d2"));
        let cue = &doc.shows[0].cues[0];
        let pan = &cue.effects[0].effect.params[0];
        assert_eq!(
            (pan.name.as_str(), &pan.remarks.comment),
            ("pan", &c("# c3"))
        );
        assert_eq!(cue.effects[0].effect.params[1].remarks, Remarks::default());
        assert_eq!(cue.effects[0].comment, None);
        let master = cue.commands[0].written.as_ref().unwrap();
        assert_eq!(master[0].remarks.comment, c("# c4"));
        assert_eq!(cue.commands[0].comment, None);
        let args = cue.sequences[0].args.as_ref().unwrap();
        assert_eq!(args[0].remarks.comment, c("# c5"));
        assert_eq!(cue.sequences[0].comment, None);
        let groups = cue.effects[1].group_remarks.as_ref().unwrap();
        assert_eq!(groups[0].comment, c("# c6"));
        assert_eq!(groups[1], Remarks::default());
        assert_eq!(cue.effects[1].comments_before, None);
    }

    #[test]
    fn a_comment_inside_a_closed_list_stays_inside_it() {
        let doc = document(
            "show \"S\" {\n    @1/1\n    sequence \"x\"(\n        # first\n        hit: 1 # on hit\n        # after\n    ) # on line\n    clear( # opened\n    )\n    offset 2 # mid\n    measures\n}\n",
            None,
        )
        .document
        .unwrap();
        let cue = &doc.shows[0].cues[0];
        let arg = &cue.sequences[0].args.as_ref().unwrap()[0].remarks;
        assert_eq!(arg.comments_before, Some(vec!["# first".to_string()]));
        assert_eq!(arg.comment.as_deref(), Some("# on hit"));
        assert_eq!(arg.comments_after, Some(vec!["# after".to_string()]));
        assert_eq!(cue.sequences[0].comment.as_deref(), Some("# on line"));
        assert_eq!(
            cue.commands[0].comments_before,
            Some(vec!["# opened".to_string()])
        );
        assert_eq!(
            cue.offsets.as_ref().unwrap()[0].comment.as_deref(),
            Some("# mid")
        );
    }

    #[test]
    fn a_tempo_start_keeps_how_it_was_written_and_what_it_read_as() {
        let doc = document("tempo {\n    start: 500ms\n}\n", None)
            .document
            .unwrap();
        let start = doc.tempo.unwrap().start;
        assert_eq!((start.value, start.unit), (500.0, DurationUnit::Ms));
        let written = start.written.unwrap();
        assert_eq!(written.text, "500ms");
        assert_eq!((written.value, written.unit), (500.0, DurationUnit::Ms));
    }

    #[test]
    fn each_offset_line_keeps_its_own_comments() {
        let doc = document(
            "show \"S\" {\n    @1/1\n    offset 1 measures # a\n    # between\n    offset 2 measures # b\n}\n",
            None,
        )
        .document
        .unwrap();
        let cue = &doc.shows[0].cues[0];
        assert_eq!(cue.offset_measures, Some(3));
        let lines = cue.offsets.as_ref().unwrap();
        assert_eq!(lines[0].measures, 1);
        assert_eq!(lines[0].comments_before, None);
        assert_eq!(lines[0].comment.as_deref(), Some("# a"));
        assert_eq!(lines[1].measures, 2);
        assert_eq!(
            lines[1].comments_before,
            Some(vec!["# between".to_string()])
        );
        assert_eq!(lines[1].comment.as_deref(), Some("# b"));
    }

    #[test]
    fn only_a_change_a_comma_follows_has_a_comment_before_its_comma() {
        let doc = document(
            "tempo {\n    changes: [\n        @2/1 { bpm: 90 } # a\n        , @3/1 { bpm: 100 } # b\n    ]\n}\n",
            None,
        )
        .document
        .unwrap();
        let changes = &doc.tempo.unwrap().changes;
        assert_eq!(changes[0].comment_before_comma, Some(true));
        assert_eq!(changes[1].comment.as_deref(), Some("# b"));
        assert_eq!(changes[1].comment_before_comma, None);
    }

    #[test]
    fn a_comment_after_a_references_arguments_is_before_its_loop() {
        let doc = document(
            "show \"S\" {\n    @1/1\n    sequence \"x\"(hit: \"#FF0000\"), # c, d\n        # above\n        loop: 2\n    sequence \"y\" # e\n        loop: 3\n}\n",
            None,
        )
        .document
        .unwrap();
        let refs = &doc.shows[0].cues[0].sequences;
        assert_eq!(
            refs[0].args.as_ref().unwrap()[0].remarks,
            Remarks::default()
        );
        assert_eq!(refs[0].loop_comment.as_deref(), Some("# c, d"));
        assert_eq!(
            refs[0].loop_comments_before,
            Some(vec!["# above".to_string()])
        );
        assert_eq!(refs[0].loop_comma, Some(true));
        assert_eq!(refs[1].loop_comment.as_deref(), Some("# e"));
        assert_eq!(refs[1].loop_comma, None);
        assert_eq!(refs[1].comments_before, None);
    }

    #[test]
    fn a_time_switched_to_another_unit_is_the_same_time() {
        for (value, from, to, want) in [
            (500.0, "ms", "s", 0.5),
            (1.5, "s", "ms", 1500.0),
            (1.001, "s", "ms", 1001.0),
            (0.1, "ms", "s", 0.0001),
            (2.0, "s", "s", 2.0),
            (0.0, "s", "ms", 0.0),
        ] {
            assert_eq!(
                time_in(value, from, to),
                Some(want),
                "{value}{from} in {to}"
            );
            assert_eq!(
                time_in(want, to, from),
                Some(value),
                "{want}{to} back in {from}"
            );
        }
        assert_eq!(time_in(1.0, "s", "beats"), None);
        assert_eq!(time_in(-1.0, "s", "ms"), None);
    }

    #[test]
    fn a_comment_after_a_bare_cue_is_the_blocks_last() {
        let doc = document(
            "show \"S\" {\n    @1/1\n    a: static, color: \"red\", duration: 1s\n\n    @2/1\n    # End in blackout\n}\n",
            None,
        )
        .document
        .unwrap();
        let show = &doc.shows[0];
        assert_eq!(show.cues[1].comment, None);
        assert_eq!(
            show.comments_after,
            Some(vec!["# End in blackout".to_string()])
        );
    }

    #[test]
    fn comment_lines_after_an_effect_with_no_duration_are_the_next_cues() {
        let doc = document(
            "show \"S\" {\n    @1/1\n    a: static, color: \"red\"\n\n    # next\n    # lines\n    @2/1\n    b: static, color: \"red\" # same line\n}\n",
            None,
        )
        .document
        .unwrap();
        let show = &doc.shows[0];
        assert_eq!(show.cues[0].effects[0].comment, None);
        assert_eq!(
            show.cues[1].comments_before,
            Some(vec!["# next".to_string(), "# lines".to_string()])
        );
        assert_eq!(
            show.cues[1].effects[0].comment.as_deref(),
            Some("# same line")
        );
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
    fn refuses_what_the_player_refuses_with_its_words() {
        let error = |source: &str, path: Option<&str>| {
            document(source, path.map(Path::new))
                .error
                .unwrap_or_else(|| panic!("{source} is refused"))
        };
        let show = "show \"S\" {\n    @1/1\n}\n";
        let player = |source: &str| {
            crate::lighting::parser::parse_light_shows(source)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
        };
        // A zero metre, a start in beats: the player's own text.
        for tempo in [
            "tempo {\n    time_signature: 0/4\n}\n",
            "tempo {\n    start: 2beats\n}\n",
        ] {
            let source = format!("{tempo}{show}");
            let refused = error(&source, Some("s.light"));
            assert!(player(&source).contains(&refused), "{refused}");
        }
        // An include with no file to resolve against, or not a `.light`.
        assert!(error(&format!("include \"a.light\"\n{show}"), None)
            .contains("resolves against the file's folder"));
        assert!(
            error(&format!("include \"a.txt\"\n{show}"), Some("s.light"))
                .contains("only .light files can be included")
        );
    }

    #[test]
    fn refuses_what_the_editor_does_not_edit() {
        let venue = "venue \"v\" {\n}\nshow \"S\" {\n    @0.0\n}\n";
        assert!(document(venue, None)
            .error
            .unwrap()
            .contains("defines a venue"));
        let two = "tempo {\n    bpm: 100\n}\ntempo {\n    bpm: 120\n}\nshow \"S\" {\n    @1/1\n}\n";
        assert!(document(two, None)
            .error
            .unwrap()
            .contains("second tempo block"));
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
