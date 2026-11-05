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

use std::error::Error;
use std::path::Path;

use pest::iterators::Pair;
use pest::Parser;
use serde::Serialize;

use super::grammar::{LightingParser, Rule};
use super::include::normalize;
use super::utils::{parse_measure_time, parse_time_string, Children};

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
    /// The comment lines after the last item, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_after: Option<Vec<String>>,
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
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoSection {
    pub start: DurationValue,
    pub bpm: f64,
    pub time_signature: (u32, u32),
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
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoComment {
    pub at: TempoField,
    pub inline: bool,
    pub text: String,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
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
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DurationValue {
    pub value: f64,
    pub unit: DurationUnit,
    /// As written (`0.0s`), for a save to keep.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub text: Option<String>,
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
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cue {
    pub timestamp: Timestamp,
    pub effects: Vec<CueEffect>,
    pub commands: Vec<LayerCommand>,
    pub sequences: Vec<SequenceRef>,
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
    /// The comment lines above its `offset` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub offset_comments_before: Option<Vec<String>>,
    /// A comment on its `offset` line.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub offset_comment: Option<String>,
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

/// The kinds of line a cue holds.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Default)]
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
#[derive(Serialize, Clone, Debug, PartialEq)]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CueEffect {
    pub groups: Vec<String>,
    pub effect: EffectParams,
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
}

/// An effect's parameters as written: each `name: value` in order, the
/// value as its text (a colour with its quotes, `50%`, `$level`), so a
/// repeated name and a spelling survive a save. The editor's forms read
/// and set them by name (`lib/lighting/params.ts`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
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
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: String,
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
    /// The comment lines above it, as written (`# …`, `// …`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comments_before: Option<Vec<String>>,
    /// A comment on its line, after it, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub comment: Option<String>,
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
    /// Whether a comma preceded `loop:`, as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub loop_comma: Option<bool>,
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
    let folder = path.and_then(Path::parent).unwrap_or(Path::new(""));
    let mut document = Document::default();
    let mut includes = Vec::new();
    let mut comments = Comments::new(source);
    // What the last item was, for a comment on its last line.
    enum Last {
        None,
        Include,
        Tempo,
        Sequence,
        Show,
    }
    let mut last = Last::None;
    let file = pairs.next().ok_or("no file")?;
    for pair in file.into_inner() {
        let line = pair.line_col().0 as u32;
        match pair.as_rule() {
            Rule::comment => {
                let Some(text) = comments.read(&pair) else {
                    continue;
                };
                let slot = match last {
                    Last::Include => includes.last_mut().map(|i: &mut Include| &mut i.comment),
                    Last::Tempo => document.tempo.as_mut().map(|t| &mut t.close_comment),
                    Last::Sequence => document.sequences.last_mut().map(|s| &mut s.close_comment),
                    Last::Show => document.shows.last_mut().map(|s| &mut s.close_comment),
                    Last::None => None,
                };
                comments.inline(slot, text);
            }
            Rule::include => {
                let written = pair
                    .clone()
                    .children()
                    .find(|p| p.as_rule() == Rule::string)
                    .map(|p| p.as_str().trim_matches('"').to_string())
                    .unwrap_or_default();
                let path = normalize(&folder.join(&written))
                    .to_string_lossy()
                    .into_owned();
                let mut include = Include {
                    written,
                    path,
                    line,
                    comments_before: comments.take(),
                    comment: None,
                };
                comments.absorb(&pair, &mut include.comment);
                includes.push(include);
                last = Last::Include;
            }
            Rule::tempo => {
                let before = comments.take();
                let mut section = tempo_section(pair, &mut comments)?;
                section.comments_before = before;
                document.tempo = Some(section);
                last = Last::Tempo;
            }
            Rule::sequence => {
                let mut seq = sequence(pair, source)?;
                seq.comments_before = comments.take();
                document.sequences.push(seq);
                last = Last::Sequence;
            }
            Rule::light_show => {
                let mut show = show(pair, source)?;
                show.comments_before = comments.take();
                document.shows.push(show);
                last = Last::Show;
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
    Ok(document)
}

/// The comments among a rule's children, kept as they are read: one on
/// the same line as what came before it is that line's; one on a line of
/// its own waits (`pending`) for the next line, or for the end of the
/// block it is in.
struct Comments<'s> {
    source: &'s str,
    pending: Vec<String>,
}

impl<'s> Comments<'s> {
    fn new(source: &'s str) -> Self {
        Comments {
            source,
            pending: Vec::new(),
        }
    }

    /// The comment's text as written; `None` after keeping it as one of
    /// its own line.
    fn read(&mut self, pair: &Pair<Rule>) -> Option<String> {
        let start = pair.as_span().start();
        let text = pair.as_str().trim_end().to_string();
        let line_start = self.source[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        if self.source[line_start..start].trim().is_empty() {
            self.pending.push(text);
            None
        } else {
            Some(text)
        }
    }

    /// Keeps a comment on a line in that line's slot; a line that holds
    /// one already, or no line at all, keeps it as a line of its own.
    fn inline(&mut self, slot: Option<&mut Option<String>>, text: String) {
        match slot {
            Some(slot @ None) => *slot = Some(text),
            _ => self.pending.push(text),
        }
    }

    /// The comment lines waiting, for the line that comes next.
    fn take(&mut self) -> Option<Vec<String>> {
        if self.pending.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.pending))
        }
    }

    /// The comments the grammar left inside an element's own pairs (a
    /// trailing one often ends up inside the last rule tried on its line):
    /// one on the element's line goes in `slot`, the rest wait.
    fn absorb(&mut self, element: &Pair<Rule>, slot: &mut Option<String>) {
        for comment in comments_in(element) {
            if let Some(text) = self.read(&comment) {
                self.inline(Some(slot), text);
            }
        }
    }
}

/// Every `comment` pair under `pair`, in order.
fn comments_in<'i>(pair: &Pair<'i, Rule>) -> Vec<Pair<'i, Rule>> {
    let mut found = Vec::new();
    for child in pair.clone().into_inner() {
        if child.as_rule() == Rule::comment {
            found.push(child);
        } else {
            found.extend(comments_in(&child));
        }
    }
    found
}

fn tempo_section(
    pair: Pair<Rule>,
    comments: &mut Comments,
) -> Result<TempoSection, Box<dyn Error>> {
    let mut section = TempoSection {
        start: DurationValue {
            value: 0.0,
            unit: DurationUnit::S,
            text: None,
        },
        bpm: crate::tempo::DEFAULT_BPM,
        time_signature: (
            crate::tempo::DEFAULT_TIME_SIGNATURE.numerator,
            crate::tempo::DEFAULT_TIME_SIGNATURE.denominator,
        ),
        changes: Vec::new(),
        comments_before: None,
        comment: None,
        comments: None,
        changes_comments_after: None,
        fields: None,
        close_comment: None,
    };
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
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::comment => {
                if let Some(text) = comments.read(&item) {
                    match last {
                        Some(at) => inner.push(TempoComment {
                            at,
                            inline: true,
                            text,
                        }),
                        None => comments.inline(Some(&mut section.comment), text),
                    }
                }
            }
            Rule::tempo_content => {
                for item in item.into_inner() {
                    match item.as_rule() {
                        Rule::comment => {
                            if let Some(text) = comments.read(&item) {
                                match last {
                                    Some(at) => inner.push(TempoComment {
                                        at,
                                        inline: true,
                                        text,
                                    }),
                                    None => comments.inline(Some(&mut section.comment), text),
                                }
                            }
                        }
                        Rule::tempo_start => {
                            place(comments.take(), TempoField::Start, &mut inner);
                            fields.push(TempoField::Start);
                            last = Some(TempoField::Start);
                            if let Some(value) = item
                                .clone()
                                .children()
                                .find(|p| p.as_rule() == Rule::time_parameter)
                            {
                                section.start = duration_value(value.as_str())?;
                            }
                            field_comments(&item, TempoField::Start, comments, &mut inner);
                        }
                        Rule::tempo_bpm => {
                            place(comments.take(), TempoField::Bpm, &mut inner);
                            fields.push(TempoField::Bpm);
                            last = Some(TempoField::Bpm);
                            if let Some(value) = item
                                .clone()
                                .children()
                                .find(|p| p.as_rule() == Rule::number_value)
                            {
                                section.bpm = value.as_str().trim().parse()?;
                            }
                            field_comments(&item, TempoField::Bpm, comments, &mut inner);
                        }
                        Rule::tempo_time_signature => {
                            place(comments.take(), TempoField::TimeSignature, &mut inner);
                            fields.push(TempoField::TimeSignature);
                            last = Some(TempoField::TimeSignature);
                            if let Some(value) = item
                                .clone()
                                .children()
                                .find(|p| p.as_rule() == Rule::time_sig_value)
                            {
                                section.time_signature = signature(value.as_str())?;
                            }
                            field_comments(&item, TempoField::TimeSignature, comments, &mut inner);
                        }
                        Rule::tempo_changes => {
                            place(comments.take(), TempoField::Changes, &mut inner);
                            fields.push(TempoField::Changes);
                            tempo_changes(item, &mut section, comments)?;
                            last = Some(TempoField::End);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
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

/// The comments the grammar left inside a tempo field's own pairs: one on
/// the field's line is the field's, the rest wait.
fn field_comments(
    item: &Pair<Rule>,
    at: TempoField,
    comments: &mut Comments,
    inner: &mut Vec<TempoComment>,
) {
    for comment in comments_in(item) {
        if let Some(text) = comments.read(&comment) {
            inner.push(TempoComment {
                at,
                inline: true,
                text,
            });
        }
    }
}

/// The `changes: [ … ]` of a tempo block, with the comments among them.
fn tempo_changes(
    pair: Pair<Rule>,
    section: &mut TempoSection,
    comments: &mut Comments,
) -> Result<(), Box<dyn Error>> {
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::comment => {
                if let Some(text) = comments.read(&item) {
                    let slot = section.changes.last_mut().map(|c| &mut c.comment);
                    comments.inline(slot, text);
                }
            }
            Rule::tempo_change_list => {
                for item in item.into_inner() {
                    match item.as_rule() {
                        Rule::comment => {
                            if let Some(text) = comments.read(&item) {
                                let slot = section.changes.last_mut().map(|c| &mut c.comment);
                                comments.inline(slot, text);
                            }
                        }
                        Rule::tempo_change => {
                            let mut change = tempo_change(item.clone())?;
                            change.comments_before = comments.take();
                            comments.absorb(&item, &mut change.comment);
                            section.changes.push(change);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    section.changes_comments_after = comments.take();
    Ok(())
}

fn tempo_change(pair: Pair<Rule>) -> Result<TempoChange, Box<dyn Error>> {
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
        comments_before: None,
        comment: None,
    };
    for item in pair.children() {
        match item.as_rule() {
            Rule::time_string | Rule::measure_time => change.timestamp = timestamp(item)?,
            Rule::tempo_change_content => {
                for param in item
                    .children()
                    .filter(|p| p.as_rule() == Rule::tempo_change_param)
                    .flat_map(Children::children)
                {
                    let rule = param.as_rule();
                    let Some(value) = param.children().next() else {
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
        text: Some(raw.trim().to_string()),
    })
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
    let mut comments = Comments::new(source);
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::comment => {
                if let Some(text) = comments.read(&item) {
                    let slot = match sequence.cues.last_mut() {
                        Some(cue) => cue.last_line(),
                        None => &mut sequence.comment,
                    };
                    comments.inline(Some(slot), text);
                }
            }
            Rule::sequence_name => sequence.name = item.as_str().trim_matches('"').to_string(),
            Rule::sequence_param_decls => {
                sequence.params = Some(
                    item.children()
                        .filter(|p| p.as_rule() == Rule::sequence_param_decl)
                        .map(arg)
                        .collect(),
                )
            }
            Rule::sequence_length => {
                sequence.length = item
                    .children()
                    .find(|p| p.as_rule() == Rule::time_parameter)
                    .map(|p| p.as_str().trim().to_string())
            }
            Rule::sequence_content => {
                let block = cues(
                    item,
                    source,
                    "a sequence",
                    &mut comments,
                    &mut sequence.comment,
                )?;
                sequence.cues = block;
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
        cues: Vec::new(),
        line: Some(line),
        comments_before: None,
        comment: None,
        comments_after: None,
        close_comment: None,
    };
    let mut comments = Comments::new(source);
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::comment => {
                if let Some(text) = comments.read(&item) {
                    let slot = match show.cues.last_mut() {
                        Some(cue) => cue.last_line(),
                        None => &mut show.comment,
                    };
                    comments.inline(Some(slot), text);
                }
            }
            Rule::show_name => show.name = item.as_str().trim_matches('"').to_string(),
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
    let mut parts = pair.children();
    SequenceArg {
        name: parts
            .next()
            .map(|p| p.as_str().trim().to_string())
            .unwrap_or_default(),
        value: parts.next().map(|p| p.as_str().trim().to_string()),
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
    for item in content.into_inner() {
        match item.as_rule() {
            Rule::comment => {
                if let Some(text) = comments.read(&item) {
                    let slot = match cues.last_mut() {
                        Some(cue) => cue.last_line(),
                        None => header,
                    };
                    comments.inline(Some(slot), text);
                }
            }
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
            CueLine::Offset => &mut self.offset_comment,
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

/// A cue as read, with the comment lines its own pairs held after its
/// last line, which belong to what follows it.
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
        reset_measures: None,
        loops: None,
        line: Some(line),
        comments_before: None,
        offset_comments_before: None,
        offset_comment: None,
        reset_comments_before: None,
        reset_comment: None,
        lines: None,
        last: CueLine::Timestamp,
    };
    let mut comments = Comments::new(source);
    let mut lines: Vec<CueLine> = Vec::new();
    for item in pair.into_inner() {
        match item.as_rule() {
            Rule::comment => {
                if let Some(text) = comments.read(&item) {
                    let slot = cue.last_line();
                    comments.inline(Some(slot), text);
                }
            }
            Rule::time_string | Rule::measure_time => cue.timestamp = timestamp(item)?,
            Rule::effect => {
                let mut effect = effect(item.clone())?;
                effect.comments_before = comments.take();
                comments.absorb(&item, &mut effect.comment);
                cue.effects.push(effect);
                cue.last = CueLine::Effect;
                lines.push(CueLine::Effect);
            }
            Rule::layer_command => {
                let mut command = layer_command(item.clone());
                command.comments_before = comments.take();
                comments.absorb(&item, &mut command.comment);
                cue.commands.push(command);
                cue.last = CueLine::Command;
                lines.push(CueLine::Command);
            }
            Rule::sequence_reference => {
                let mut reference = sequence_ref(item.clone());
                reference.comments_before = comments.take();
                comments.absorb(&item, &mut reference.comment);
                cue.sequences.push(reference);
                cue.last = CueLine::Reference;
                lines.push(CueLine::Reference);
            }
            Rule::stop_sequence_command => {
                let name = item
                    .clone()
                    .children()
                    .find(|p| p.as_rule() == Rule::sequence_name)
                    .map(|p| p.as_str().trim_matches('"').to_string())
                    .unwrap_or_default();
                let mut stop = SequenceRef {
                    name,
                    args: None,
                    loop_: None,
                    stop: Some(true),
                    loop_comma: None,
                    comments_before: comments.take(),
                    comment: None,
                };
                comments.absorb(&item, &mut stop.comment);
                cue.sequences.push(stop);
                cue.last = CueLine::Reference;
                lines.push(CueLine::Reference);
            }
            Rule::offset_command => {
                let measures: u32 = item
                    .clone()
                    .children()
                    .find(|p| p.as_rule() == Rule::number_value)
                    .map(|p| p.as_str().trim().parse())
                    .transpose()?
                    .unwrap_or(0);
                cue.offset_measures = Some(cue.offset_measures.unwrap_or(0) + measures);
                if let Some(pending) = comments.take() {
                    cue.offset_comments_before
                        .get_or_insert_with(Vec::new)
                        .extend(pending);
                }
                comments.absorb(&item, &mut cue.offset_comment);
                cue.last = CueLine::Offset;
                lines.push(CueLine::Offset);
            }
            Rule::reset_measures_command => {
                cue.reset_measures = Some(true);
                if let Some(pending) = comments.take() {
                    cue.reset_comments_before
                        .get_or_insert_with(Vec::new)
                        .extend(pending);
                }
                comments.absorb(&item, &mut cue.reset_comment);
                cue.last = CueLine::Reset;
                lines.push(CueLine::Reset);
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
    // A comment line the cue's own pairs hold after its last line belongs
    // to what follows the cue: handed back to the body.
    Ok((cue, comments.take()))
}

fn effect(pair: Pair<Rule>) -> Result<CueEffect, Box<dyn Error>> {
    let source = pair.as_str();
    let base = pair.as_span().start();
    let mut groups = Vec::new();
    let mut params = EffectParams {
        kind: EffectKind::Static,
        params: Vec::new(),
        no_comma: None,
    };
    let mut type_end = None;
    for item in pair.children() {
        match item.as_rule() {
            Rule::group_list => {
                groups.extend(item.children().map(|g| g.as_str().trim().to_string()))
            }
            Rule::effect_type => {
                type_end = Some(item.as_span().end() - base);
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
                if let Some(end) = type_end {
                    let between = &source[end..item.as_span().start() - base];
                    if !between.contains(',') {
                        params.no_comma = Some(true);
                    }
                }
                for parameter in item.children().filter(|p| p.as_rule() == Rule::parameter) {
                    let mut parts = parameter.children();
                    let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    params.params.push(Param {
                        name: name.as_str().trim().to_string(),
                        value: value.as_str().trim().to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(CueEffect {
        groups,
        effect: params,
        comments_before: None,
        comment: None,
    })
}

fn layer_command(pair: Pair<Rule>) -> LayerCommand {
    let mut command = LayerCommand {
        command: LayerCommandKind::Clear,
        layer: None,
        intensity: None,
        speed: None,
        comments_before: None,
        comment: None,
    };
    for item in pair.children() {
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
                    .children()
                    .filter(|p| p.as_rule() == Rule::layer_command_param)
                {
                    let mut parts = param.children();
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
        loop_comma: None,
        comments_before: None,
        comment: None,
    };
    let source = pair.as_str();
    let base = pair.as_span().start();
    let mut before_params = 0;
    for item in pair.children() {
        match item.as_rule() {
            Rule::sequence_name => {
                reference.name = item.as_str().trim_matches('"').to_string();
                before_params = item.as_span().end() - base;
            }
            Rule::sequence_args => {
                before_params = item.as_span().end() - base;
                reference.args = Some(
                    item.children()
                        .filter(|p| p.as_rule() == Rule::sequence_arg)
                        .map(arg)
                        .collect(),
                )
            }
            Rule::sequence_params => {
                if source[before_params..item.as_span().start() - base].contains(',') {
                    reference.loop_comma = Some(true);
                }
                for param in item
                    .children()
                    .filter(|p| p.as_rule() == Rule::sequence_param)
                {
                    let mut parts = param.children();
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
        let doc = document(source, None).document.expect("parses");
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
        assert_eq!(cue.offset_comment.as_deref(), Some("# on offset"));
        assert_eq!(cue.reset_comment.as_deref(), Some("# on reset"));
        let second = &show.cues[1];
        assert_eq!(second.comments_before, v(&["# between cues"]));
        assert_eq!(
            second.loops.as_ref().unwrap()[0].comment.as_deref(),
            Some("# on loop")
        );
        assert_eq!(doc.comments_after, v(&["# file end"]));
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
