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
//! A [`Document`] written back as DSL text: the one writer of the light
//! show grammar (`grammar.pest` is its one reader). What the document
//! holds as written -- a spelling, an order, a comment -- is written so;
//! what an edit set is written in the grammar's own form. An untouched
//! document writes the file it was read from, whitespace aside; every
//! comment is written where it was, a removed element's kept in its place
//! (see [`dropped`]).
//!
//! The editor never builds DSL text: it edits the document (see
//! [`super::edit`]) and asks for it written here.
//!
//! A word the parser reads by its own table is written from that table
//! (effect and layer-command keywords, layer-command parameters, units,
//! tempo fields, transitions). The words spelled by `grammar.pest` alone,
//! with no table on the parser's side, are written here as the grammar
//! spells them: `include`, `show`, `tempo`, `sequence`, `length`,
//! `offset … measures`, `reset_measures`, `stop` and `loop`.

use super::super::tempo::{
    parse_signature_in, SignatureRange, TempoTransition, DEFAULT_BPM, DEFAULT_TIME_SIGNATURE,
};
use super::document::{
    Cue, CueEffect, CueLine, Document, DocumentItem, DurationUnit, LayerCommand, LayerCommandParam,
    OffsetLine, Param, Remarks, SequenceArg, SequenceRef, TempoChange, TempoField, TempoSection,
    Timestamp, TimestampKind, TRANSITION,
};
use super::tempo_parse::{TRANSITION_MEASURES, TRANSITION_SNAP};

/// The document as DSL text.
pub fn write(file: &Document) -> String {
    let mut parts: Vec<String> = Vec::new();
    let includes = file.includes.as_deref().unwrap_or_default();

    // The items in the order the file wrote them, while the lists still
    // match it (an edit that adds or removes an item writes them grouped).
    let counts = [
        (DocumentItem::Include, includes.len()),
        (DocumentItem::Tempo, usize::from(file.tempo.is_some())),
        (DocumentItem::Sequence, file.sequences.len()),
        (DocumentItem::Show, file.shows.len()),
    ];
    let written = file.items.as_deref().unwrap_or_default();
    let order: Vec<DocumentItem> = if as_written(written, &counts) {
        written.to_vec()
    } else {
        grouped(&counts)
    };
    let mut next = [0usize; 4];
    for (i, kind) in order.iter().enumerate() {
        let slot = &mut next[*kind as usize];
        let n = *slot;
        *slot += 1;
        match kind {
            DocumentItem::Include => {
                let inc = &includes[n];
                parts.extend(own(&inc.comments_before, ""));
                parts.push(line(format!("include \"{}\"", inc.written), &inc.comment));
                // Includes sit together, one blank line after the run.
                if order.get(i + 1) != Some(&DocumentItem::Include) {
                    parts.push(String::new());
                }
            }
            DocumentItem::Tempo => {
                if let Some(tempo) = &file.tempo {
                    parts.push(write_tempo(tempo));
                }
                parts.push(String::new());
            }
            DocumentItem::Sequence => {
                let seq = &file.sequences[n];
                let params = seq
                    .params
                    .as_ref()
                    .map(|p| arg_list(p, "    "))
                    .unwrap_or_default();
                let length = match seq.length.as_deref() {
                    Some(l) if !l.is_empty() => format!(", length: {l}"),
                    _ => String::new(),
                };
                parts.extend(own(&seq.comments_before, ""));
                parts.push(line(
                    format!("sequence \"{}\"{params}{length} {{", seq.name),
                    &seq.comment,
                ));
                parts.push(cues(&seq.cues));
                parts.extend(own(&seq.comments_after, "    "));
                parts.push(line("}".to_string(), &seq.close_comment));
                parts.push(String::new());
            }
            DocumentItem::Show => {
                let show = &file.shows[n];
                parts.extend(own(&show.comments_before, ""));
                let head = if show.unnamed == Some(true) && show.name.is_empty() {
                    "show {".to_string()
                } else {
                    format!("show \"{}\" {{", show.name)
                };
                parts.push(line(head, &show.comment));
                parts.push(cues(&show.cues));
                parts.extend(own(&show.comments_after, "    "));
                parts.push(line("}".to_string(), &show.close_comment));
                parts.push(String::new());
            }
        }
    }

    parts.extend(own(&file.comments_after, ""));
    if file.comments_after.as_ref().is_some_and(|c| !c.is_empty()) {
        parts.push(String::new());
    }
    parts.join("\n")
}

/// Whether `written` holds each kind as many times as `counts` says.
fn as_written<K: PartialEq + Copy>(written: &[K], counts: &[(K, usize)]) -> bool {
    counts
        .iter()
        .all(|(kind, n)| written.iter().filter(|k| *k == kind).count() == *n)
}

/// Each kind `counts` times, in `counts`' order.
fn grouped<K: Copy>(counts: &[(K, usize)]) -> Vec<K> {
    counts
        .iter()
        .flat_map(|(kind, n)| std::iter::repeat_n(*kind, *n))
        .collect()
}

/// A comment as the document holds it, when it holds one.
fn some(comment: &Option<String>) -> Option<&str> {
    comment.as_deref().filter(|c| !c.is_empty())
}

/// A line with the comment that was on it, if one was.
fn line(text: String, comment: &Option<String>) -> String {
    match some(comment) {
        Some(c) => format!("{text} {c}"),
        None => text,
    }
}

/// Comment lines of their own, as written, at an indent.
fn own<'a>(
    comments: &'a Option<Vec<String>>,
    indent: &'a str,
) -> impl Iterator<Item = String> + 'a {
    comments
        .iter()
        .flatten()
        .map(move |c| format!("{indent}{c}"))
}

/// What carries the comments written around it: an element of a list.
pub(super) trait Remarked {
    fn remarks(&self) -> &Remarks;
    fn remarks_mut(&mut self) -> &mut Remarks;
}

impl Remarked for Param {
    fn remarks(&self) -> &Remarks {
        &self.remarks
    }
    fn remarks_mut(&mut self) -> &mut Remarks {
        &mut self.remarks
    }
}

impl Remarked for SequenceArg {
    fn remarks(&self) -> &Remarks {
        &self.remarks
    }
    fn remarks_mut(&mut self) -> &mut Remarks {
        &mut self.remarks
    }
}

/// An element of a list (a parameter, an argument, a group) as text, with
/// the comments written around it.
#[derive(Clone, Default)]
struct Listed {
    text: String,
    remarks: Remarks,
}

impl Remarked for Listed {
    fn remarks(&self) -> &Remarks {
        &self.remarks
    }
    fn remarks_mut(&mut self) -> &mut Remarks {
        &mut self.remarks
    }
}

/// `items` without those `drop` picks, the comments written around each
/// one dropped kept as comment lines where it was: above the next item
/// kept, else after the one before. The second list: those with no item
/// left to go to.
pub(super) fn dropped<T: Remarked>(
    items: Vec<T>,
    drop: impl Fn(&T, usize) -> bool,
) -> (Vec<T>, Vec<String>) {
    let mut kept: Vec<T> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for (i, mut item) in items.into_iter().enumerate() {
        if drop(&item, i) {
            let r = item.remarks();
            pending.extend(r.comments_before.iter().flatten().cloned());
            pending.extend(some(&r.comment).map(str::to_string));
            pending.extend(r.comments_after.iter().flatten().cloned());
        } else if !pending.is_empty() {
            let r = item.remarks_mut();
            pending.extend(r.comments_before.take().into_iter().flatten());
            r.comments_before = Some(std::mem::take(&mut pending));
            kept.push(item);
        } else {
            kept.push(item);
        }
    }
    if !pending.is_empty() {
        if let Some(last) = kept.last_mut() {
            let r = last.remarks_mut();
            let mut after = r.comments_after.take().unwrap_or_default();
            after.append(&mut pending);
            r.comments_after = Some(after);
        }
    }
    (kept, pending)
}

/// A list's elements, `, `-separated on one line, each with its comments
/// where they were: a comment after an element ends the line after its
/// comma, and comment lines above an element or after the last are lines
/// of their own; a line the list goes on to starts at `indent`. `space`:
/// whether the first element follows a space. The second value is whether
/// the text ends a line, so what closes the list starts the next one.
fn list(items: &[Listed], indent: &str, space: bool) -> (String, bool) {
    let mut text = String::new();
    let mut open = false;
    let newline = |text: &mut String, open: &mut bool| {
        text.push('\n');
        text.push_str(indent);
        *open = true;
    };
    for (i, item) in items.iter().enumerate() {
        let r = &item.remarks;
        match r.comments_before.as_deref() {
            Some(before) if !before.is_empty() => {
                if !open {
                    newline(&mut text, &mut open);
                }
                for c in before {
                    text.push_str(c);
                    newline(&mut text, &mut open);
                }
            }
            _ => {
                if !open && (i > 0 || space) {
                    text.push(' ');
                }
            }
        }
        text.push_str(&item.text);
        if i + 1 < items.len() {
            text.push(',');
        }
        open = false;
        if let Some(c) = some(&r.comment) {
            text.push(' ');
            text.push_str(c);
            newline(&mut text, &mut open);
        }
        for c in r.comments_after.iter().flatten() {
            if !open {
                newline(&mut text, &mut open);
            }
            text.push_str(c);
            newline(&mut text, &mut open);
        }
    }
    (text, open)
}

/// A list that nothing closes, as the end of its line: one that ended a
/// line ends there.
fn ended((text, open): (String, bool), indent: &str) -> String {
    if open {
        text[..text.len() - (indent.len() + 1)].to_string()
    } else {
        text
    }
}

/// A parameter as written, with its comments.
fn param(p: &Param) -> Listed {
    Listed {
        text: format!("{}: {}", p.name, p.value),
        remarks: p.remarks.clone(),
    }
}

/// The `tempo { }` block alone, as [`write`] writes it: what the editor's
/// tempo questions read a section through, so they go to the parser's own
/// reading of the block it will write.
pub fn write_tempo(tempo: &TempoSection) -> String {
    let mut lines: Vec<String> = Vec::new();
    let comments = tempo.comments.as_deref().unwrap_or_default();
    let at = |field: TempoField, inline: bool| -> Vec<String> {
        comments
            .iter()
            .filter(|c| c.at == field && c.inline == inline)
            .map(|c| c.text.clone())
            .collect()
    };
    let inline = |field: TempoField| Some(at(field, true).join(" "));
    // A field's line: its name and its value, with its comments.
    let field = |lines: &mut Vec<String>, name: TempoField, value: String| {
        let above = Some(at(name, false));
        lines.extend(own(&above, "    "));
        lines.push(line(
            format!("    {}: {value}", name.keyword()),
            &inline(name),
        ));
    };
    lines.extend(own(&tempo.comments_before, ""));
    lines.push(line("tempo {".to_string(), &tempo.comment));
    // The fields in the order the block wrote them; one it left out is
    // written after them only once it is set to something other than the
    // parser's default for it (a block with no `start:` stays without one).
    let mut order: Vec<TempoField> = tempo.fields.clone().unwrap_or_default();
    let changes_unset = tempo.changes.is_empty()
        && tempo
            .changes_comments_after
            .as_ref()
            .is_none_or(Vec::is_empty);
    for f in [
        TempoField::Start,
        TempoField::Bpm,
        TempoField::TimeSignature,
        TempoField::Changes,
    ] {
        let unset = match f {
            TempoField::Start => tempo.start.value == 0.0,
            TempoField::Bpm => tempo.bpm == DEFAULT_BPM,
            TempoField::TimeSignature => {
                tempo.time_signature
                    == (
                        DEFAULT_TIME_SIGNATURE.numerator,
                        DEFAULT_TIME_SIGNATURE.denominator,
                    )
            }
            _ => changes_unset,
        };
        let left_out = tempo.fields.is_some() && unset;
        if !order.contains(&f) && !left_out {
            order.push(f);
        }
    }
    for name in order {
        match name {
            TempoField::Start => {
                // As written, while the value and the unit are still the
                // ones read.
                let start = &tempo.start;
                let text = match &start.written {
                    Some(w) if w.value == start.value && w.unit == start.unit => w.text.clone(),
                    _ => length_text(start.value, start.unit),
                };
                field(&mut lines, name, text);
            }
            TempoField::Bpm => {
                let text = match tempo.bpm_text.as_deref() {
                    Some(t) if t.parse() == Ok(tempo.bpm) => t.to_string(),
                    _ => number(tempo.bpm),
                };
                field(&mut lines, name, text);
            }
            TempoField::TimeSignature => {
                let text = match tempo.time_signature_text.as_deref() {
                    Some(t) if reads_as(t, tempo.time_signature) => t.to_string(),
                    _ => signature(tempo.time_signature),
                };
                field(&mut lines, name, text);
            }
            TempoField::Changes => {
                if changes_unset {
                    continue;
                }
                field(&mut lines, name, "[".to_string());
                let count = tempo.changes.len();
                for (i, change) in tempo.changes.iter().enumerate() {
                    let ts = timestamp(&change.timestamp);
                    let (params, orphans) = change_params(change);
                    let (params, open) = list(&params, "            ", true);
                    let last = i + 1 == count;
                    lines.extend(own(&change.comments_before, "        "));
                    lines.extend(orphans.iter().map(|c| format!("        {c}")));
                    let text = format!("        {ts} {{{params}{}", if open { "}" } else { " }" });
                    if !last
                        && some(&change.comment).is_some()
                        && change.comment_before_comma == Some(true)
                    {
                        // The comment was between the `}` and the comma: it
                        // stays there.
                        lines.push(line(text, &change.comment));
                        lines.push("        ,".to_string());
                    } else {
                        let comma = if last { "" } else { "," };
                        lines.push(line(format!("{text}{comma}"), &change.comment));
                    }
                }
                lines.extend(own(&tempo.changes_comments_after, "        "));
                lines.push(line("    ]".to_string(), &inline(TempoField::End)));
            }
            TempoField::End => {}
        }
    }
    lines.extend(own(&Some(at(TempoField::End, false)), "    "));
    lines.push(line("}".to_string(), &tempo.close_comment));
    lines.join("\n")
}

/// A tempo change's parameters: as written, in order, spelling and with
/// their comments, an edited one's value replaced where it was (see
/// [`edited`]).
fn change_params(change: &TempoChange) -> (Vec<Listed>, Vec<String>) {
    edited(
        change.written.as_deref().unwrap_or_default(),
        &[
            (TempoField::Bpm.keyword(), change.bpm.map(number)),
            (
                TempoField::TimeSignature.keyword(),
                change.time_signature.map(signature),
            ),
            (TRANSITION, change.transition.clone()),
        ],
        |p| {
            if p.name == TempoField::Bpm.keyword() {
                change.bpm.is_some_and(|bpm| p.value.parse() == Ok(bpm))
            } else if p.name == TempoField::TimeSignature.keyword() {
                change
                    .time_signature
                    .is_some_and(|sig| reads_as(&p.value, sig))
            } else {
                p.name == TRANSITION && change.transition.as_deref() == Some(p.value.as_str())
            }
        },
    )
}

/// A metre in the grammar's form: `3/4`.
fn signature((n, d): (u32, u32)) -> String {
    format!("{n}/{d}")
}

/// Whether a metre as written reads, as the player reads it, as `sig`.
fn reads_as(written: &str, sig: (u32, u32)) -> bool {
    parse_signature_in(written, SignatureRange::Light)
        .is_ok_and(|read| (read.numerator, read.denominator) == sig)
}

/// Written parameters with the values now `set`: each as written while
/// it `holds` (its value still the one set), else its value replaced in
/// place -- the last of a name, which is the one that counts; a name set
/// to nothing is dropped, its comments kept where it was (see [`dropped`];
/// the second list when no parameter is left), and one newly set is
/// added at the end.
fn edited(
    written: &[Param],
    set: &[(&str, Option<String>)],
    holds: impl Fn(&Param) -> bool,
) -> (Vec<Listed>, Vec<String>) {
    let value = |name: &str| -> Option<&str> {
        set.iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, v)| v.as_deref())
            .filter(|v| !v.is_empty())
    };
    let last = |name: &str| written.iter().rposition(|p| p.name == name);
    let replaced: Vec<Param> = written
        .iter()
        .enumerate()
        .map(|(i, p)| match value(&p.name) {
            Some(v) if last(&p.name) == Some(i) && !holds(p) => Param {
                value: v.to_string(),
                ..p.clone()
            },
            _ => p.clone(),
        })
        .collect();
    let (kept, orphans) = dropped(replaced, |p, _| value(&p.name).is_none());
    let mut params: Vec<Listed> = kept.iter().map(param).collect();
    for (name, _) in set {
        if let Some(v) = value(name) {
            if last(name).is_none() {
                params.push(Listed {
                    text: format!("{name}: {v}"),
                    remarks: Remarks::default(),
                });
            }
        }
    }
    (params, orphans)
}

/// A show's or a sequence's cues, a blank line between each two.
fn cues(cues: &[Cue]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (i, cue) in cues.iter().enumerate() {
        lines.extend(own(&cue.comments_before, "    "));
        lines.push(line(
            format!("    {}", timestamp(&cue.timestamp)),
            &cue.comment,
        ));
        cue_lines(cue, &mut lines);
        if i + 1 < cues.len() {
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

/// A cue's lines, each kind in its list's order, in the order the file
/// wrote them while the lists still match that order (an edit that adds
/// or removes a line writes the kinds grouped).
fn cue_lines(cue: &Cue, lines: &mut Vec<String>) {
    // The `offset` lines as written, each with its comments, while they
    // still add up; an edited sum is one line, the comments kept on it.
    let read = cue.offsets.as_deref().unwrap_or_default();
    let sum: u64 = read.iter().map(|l| u64::from(l.measures)).sum();
    let offsets: Vec<OffsetLine> = match cue.offset_measures {
        Some(m) if !read.is_empty() && sum == u64::from(m) => read.to_vec(),
        Some(m) => {
            let mut before = Vec::new();
            for (i, l) in read.iter().enumerate() {
                before.extend(l.comments_before.iter().flatten().cloned());
                if i + 1 < read.len() {
                    before.extend(some(&l.comment).map(str::to_string));
                }
            }
            vec![OffsetLine {
                measures: m,
                comments_before: Some(before),
                comment: read.last().and_then(|l| l.comment.clone()),
            }]
        }
        None => Vec::new(),
    };
    // Offset lines all removed leave their comments where they were.
    let removed: &[OffsetLine] = if cue.offset_measures.is_none() {
        read
    } else {
        &[]
    };
    let loops = cue.loops.as_deref().unwrap_or_default();
    let counts = [
        (CueLine::Effect, cue.effects.len()),
        (CueLine::Command, cue.commands.len()),
        (CueLine::Reference, cue.sequences.len()),
        (CueLine::Offset, offsets.len() + removed.len()),
        (
            CueLine::Reset,
            usize::from(cue.reset_measures == Some(true)),
        ),
        (CueLine::Loop, loops.len()),
    ];
    let written = cue.lines.as_deref().unwrap_or_default();
    let order: Vec<CueLine> = if as_written(written, &counts) {
        written.to_vec()
    } else {
        grouped(&counts)
    };
    let mut next = [0usize; 7];
    for kind in order {
        let slot = &mut next[kind as usize];
        let i = *slot;
        *slot += 1;
        match kind {
            CueLine::Effect => {
                let eff = &cue.effects[i];
                let Some((text, open)) = effect(eff, "        ") else {
                    continue;
                };
                lines.extend(own(&eff.comments_before, "    "));
                match some(&eff.comment) {
                    // Its last parameter's comment ends the line: the
                    // effect's own comment is a line of its own after it,
                    // never merged into it.
                    Some(c) if open => {
                        lines.push(format!("    {text}"));
                        lines.push(format!("    {c}"));
                    }
                    _ => lines.push(line(format!("    {text}"), &eff.comment)),
                }
                lines.extend(own(&eff.comments_after, "    "));
            }
            CueLine::Command => {
                let cmd = &cue.commands[i];
                let (text, orphans) = layer_command(cmd, "        ");
                lines.extend(own(&cmd.comments_before, "    "));
                lines.extend(orphans.iter().map(|c| format!("    {c}")));
                lines.push(line(format!("    {text}"), &cmd.comment));
            }
            CueLine::Reference => {
                let r = &cue.sequences[i];
                lines.extend(own(&r.comments_before, "    "));
                lines.push(line(
                    format!("    {}", sequence_ref(r, "        ")),
                    &r.comment,
                ));
            }
            CueLine::Offset => {
                if let Some(gone) = removed.get(i) {
                    lines.extend(own(&gone.comments_before, "    "));
                    lines.extend(some(&gone.comment).map(|c| format!("    {c}")));
                    continue;
                }
                let offset = &offsets[i];
                lines.extend(own(&offset.comments_before, "    "));
                lines.push(line(
                    format!("    offset {} measures", offset.measures),
                    &offset.comment,
                ));
            }
            CueLine::Reset => {
                lines.extend(own(&cue.reset_comments_before, "    "));
                lines.push(line("    reset_measures".to_string(), &cue.reset_comment));
            }
            CueLine::Loop => {
                // Inline loops, kept as the text they were read from.
                let l = &loops[i];
                lines.extend(own(&l.comments_before, "    "));
                lines.push(line(format!("    {}", l.text), &l.comment));
            }
            CueLine::Timestamp => {}
        }
    }
}

/// A timestamp as written: its text while it is the one read (`@1/1.5`),
/// else in the grammar's own form (`@3/1.5`, `@MM:SS.mmm` in whole
/// milliseconds -- the document holds whole ones; an edit's fraction is
/// rounded away).
pub fn timestamp(ts: &Timestamp) -> String {
    if let Some(text) = ts.text.as_deref().filter(|t| !t.is_empty()) {
        return text.to_string();
    }
    if ts.kind == TimestampKind::MeasureBeat {
        let beat = ts.beat.map(number).unwrap_or_else(|| "1".to_string());
        return format!("@{}/{beat}", ts.measure.unwrap_or(1));
    }
    let total = round(ts.ms.unwrap_or(0.0)).max(0.0) as u64;
    format!(
        "@{:02}:{:02}.{:03}",
        total / 60000,
        (total % 60000) / 1000,
        total % 1000
    )
}

/// An effect line; a line it goes on to starts at `indent`. The flag:
/// whether its last parameter's comment ends its last line. `None` for an
/// effect with no group left to name.
fn effect(eff: &CueEffect, indent: &str) -> Option<(String, bool)> {
    // A group removed leaves its comments where it was (see `dropped`).
    let remarks = eff.group_remarks.as_deref().unwrap_or_default();
    let mut groups: Vec<Listed> = eff
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| Listed {
            text: g.clone(),
            remarks: remarks.get(i).cloned().unwrap_or_default(),
        })
        .collect();
    groups.extend(remarks.iter().skip(eff.groups.len()).map(|r| Listed {
        text: String::new(),
        remarks: r.clone(),
    }));
    let (named, _) = dropped(groups, |g, _| g.text.is_empty());
    if named.is_empty() {
        return None;
    }
    let (groups, _) = list(&named, indent, false);
    let head = format!("{groups}: {}", eff.effect.kind.keyword());
    if eff.effect.params.is_empty() {
        return Some((head, false));
    }
    let params: Vec<Listed> = eff.effect.params.iter().map(param).collect();
    let listed = list(&params, indent, true);
    let open = listed.1;
    let comma = if eff.effect.no_comma == Some(true) {
        ""
    } else {
        ","
    };
    Some((format!("{head}{comma}{}", ended(listed, indent)), open))
}

/// A layer command, and the comments of parameters it no longer has when
/// none is left to keep them (see [`edited`]).
fn layer_command(cmd: &LayerCommand, indent: &str) -> (String, Vec<String>) {
    // As written, an edited value replaced where it was.
    let set =
        LayerCommandParam::ALL.map(|(param, name)| (name, cmd.param(param).map(str::to_string)));
    let (params, orphans) = edited(cmd.written.as_deref().unwrap_or_default(), &set, |p| {
        set.iter()
            .find(|(n, _)| *n == p.name)
            .is_some_and(|(_, v)| v.as_deref() == Some(p.value.as_str()))
    });
    let (text, _) = list(&params, indent, false);
    (format!("{}({text})", cmd.command.keyword()), orphans)
}

fn sequence_ref(r: &SequenceRef, indent: &str) -> String {
    let stop = r.stop == Some(true);
    let mut out = String::new();
    if stop {
        out.push_str("stop ");
    }
    out.push_str(&format!("sequence \"{}\"", r.name));
    if let (Some(args), false) = (&r.args, stop) {
        out.push_str(&arg_list(args, indent));
    }
    if let (Some(count), false) = (r.loop_.as_deref().filter(|l| !l.is_empty()), stop) {
        if r.loop_comma == Some(true) {
            out.push(',');
        }
        // A comment after the arguments, and comment lines before `loop:`,
        // put `loop:` on a line of its own.
        let before = r.loop_comments_before.as_deref().unwrap_or_default();
        if some(&r.loop_comment).is_some() || !before.is_empty() {
            if let Some(c) = some(&r.loop_comment) {
                out.push_str(&format!(" {c}"));
            }
            for c in before {
                out.push_str(&format!("\n{indent}{c}"));
            }
            out.push_str(&format!("\n{indent}loop: {count}"));
        } else {
            out.push_str(&format!(" loop: {count}"));
        }
    }
    out
}

/// `(name: value, …)`; a parameter with no default is its name alone.
fn arg_list(args: &[SequenceArg], indent: &str) -> String {
    let items: Vec<Listed> = args
        .iter()
        .map(|a| Listed {
            text: match &a.value {
                None => a.name.clone(),
                Some(v) => format!("{}: {v}", a.name),
            },
            remarks: a.remarks.clone(),
        })
        .collect();
    format!("({})", list(&items, indent, false).0)
}

/// A length in the grammar's form: the number and its unit (`2beats`,
/// `1measure`, `0.5s`).
pub fn length_text(value: f64, unit: DurationUnit) -> String {
    format!("{}{}", number(value), unit.keyword(value))
}

/// A length the timeline dragged, as the DSL writes it: whole measures
/// when it is whole bars of the metres it spans (`span`: its measures and
/// beats, when a tempo map counts them), else whole beats, else time.
pub fn duration_text(span: Option<(f64, f64)>, ms: f64) -> String {
    let whole = |n: f64| n >= 1.0 && (n - round(n)).abs() < 0.001;
    if let Some((measures, beats)) = span {
        if whole(measures) {
            return length_text(round(measures), DurationUnit::Measures);
        }
        if whole(beats) {
            return length_text(round(beats), DurationUnit::Beats);
        }
    }
    if ms < 1000.0 || ms % 1000.0 != 0.0 {
        return length_text(round(ms), DurationUnit::Ms);
    }
    length_text(ms / 1000.0, DurationUnit::S)
}

/// A tempo change's `transition:` value as the grammar writes it: `4`
/// (beats), `2m` (measures) or `snap`.
pub fn transition_text(transition: &TempoTransition) -> String {
    match transition {
        TempoTransition::Snap => TRANSITION_SNAP.to_string(),
        TempoTransition::Beats(n, _) => number(*n),
        TempoTransition::Measures(n, _) => format!("{}{TRANSITION_MEASURES}", number(*n)),
    }
}

/// A number as the editor has always written one (JavaScript's
/// `Number.prototype.toString`): the shortest decimal that reads back as
/// it, in exponent form below 1e-6 and from 1e21.
pub fn number(n: f64) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if n.abs() >= 1e21 || n.abs() < 1e-6 {
        let text = format!("{n:e}");
        return match text.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => text,
        };
    }
    format!("{n}")
}

/// `Math.round`: to the nearest integer, a half up.
fn round(n: f64) -> f64 {
    (n + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::document::{
        document, Document, DurationValue, TimestampKind, WrittenDuration,
    };
    use super::super::edit::EffectEdit;
    use super::*;

    fn read(source: &str) -> Document {
        let read = document(source, None);
        read.document
            .unwrap_or_else(|| panic!("{:?}\n{source}", read.error))
    }

    /// The document with what the text's layout alone decides (its line
    /// numbers) left out.
    fn unlined(doc: &Document) -> serde_json::Value {
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(map) => {
                    map.remove("line");
                    map.values_mut().for_each(strip);
                }
                serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
                _ => {}
            }
        }
        let mut v = serde_json::to_value(doc).unwrap();
        strip(&mut v);
        v
    }

    /// The comment on each line, as written: what must survive a save.
    fn comment_lines(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for raw in text.lines() {
            let l = raw.trim();
            let mut quoted = false;
            let b = l.as_bytes();
            for i in 0..b.len() {
                if b[i] == b'"' {
                    quoted = !quoted;
                }
                if !quoted && (b[i] == b'#' || (b[i] == b'/' && b.get(i + 1) == Some(&b'/'))) {
                    out.push(l[i..].trim_end().to_string());
                    break;
                }
            }
        }
        out
    }

    /// The text's lines, whitespace aside.
    fn words(text: &str) -> Vec<String> {
        text.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// `source`'s first show with its first cue edited, written out.
    fn edit_cue(source: &str, edit: impl Fn(&mut Cue)) -> String {
        let mut doc = read(source);
        edit(&mut doc.shows[0].cues[0]);
        write(&doc)
    }

    /// `source` with its first effect edited by `edit`, written out.
    fn edit_effect(source: &str, edit: EffectEdit) -> String {
        edit_cue(source, |cue| cue.effects[0] = cue.effects[0].edited(&edit))
    }

    fn set(name: &str, value: Option<&str>) -> EffectEdit {
        EffectEdit::Set {
            name: name.to_string(),
            value: value.map(str::to_string),
        }
    }

    fn show(body: &str) -> String {
        format!("show \"S\" {{\n    @1/1\n{body}}}\n")
    }

    /// Every `.light` file the round trip is held to: the examples, and
    /// with `MTRACK_LIGHT_CORPUS=<dir>` every file in a folder.
    fn corpus() -> Vec<PathBuf> {
        let mut folders =
            vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/lighting/shows")];
        if let Ok(dir) = std::env::var("MTRACK_LIGHT_CORPUS") {
            folders.push(PathBuf::from(dir));
        }
        let mut files = Vec::new();
        for folder in folders {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "light") {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn every_file_writes_back_as_read() {
        let mut held = 0;
        for path in corpus() {
            let source = std::fs::read_to_string(&path).unwrap();
            let Some(doc) = document(&source, Some(&path)).document else {
                continue; // Not an editor's file (a venue, a show's own tempo).
            };
            held += 1;
            let written = write(&doc);
            let again = document(&written, Some(&path));
            let again = again
                .document
                .unwrap_or_else(|| panic!("{}: {:?}\n{written}", path.display(), again.error));
            assert_eq!(unlined(&again), unlined(&doc), "{}", path.display());
            // Every comment comes back, as written and in order, and the
            // bytes differ in whitespace alone.
            assert_eq!(
                comment_lines(&written),
                comment_lines(&source),
                "{}",
                path.display()
            );
            // Written again, it is the same text.
            assert_eq!(write(&again), written, "{}", path.display());
            if words(&source) != words(&written) {
                // A file's own spacing inside a line (`a,b`) is the one
                // difference allowed: the same tokens, line by line.
                let squash = |t: &str| t.split_whitespace().collect::<String>();
                assert_eq!(squash(&source), squash(&written), "{}", path.display());
            }
        }
        assert!(held > 0);
    }

    #[test]
    fn a_time_of_whole_milliseconds_is_written_as_read() {
        for written in ["@1.005", "@0.001", "@00:00.003", "@2:03.5"] {
            let source = format!(
                "show \"S\" {{\n    {written}\n    a: static, color: \"red\", duration: 1s\n}}\n"
            );
            let doc = read(&source);
            let again = read(&write(&doc));
            assert_eq!(
                again.shows[0].cues[0].timestamp,
                doc.shows[0].cues[0].timestamp
            );
        }
    }

    #[test]
    fn an_edited_timestamp_is_written_in_the_grammars_form() {
        let ts = |kind, ms, measure, beat| Timestamp {
            kind,
            text: None,
            ms,
            measure,
            beat,
        };
        assert_eq!(
            timestamp(&ts(TimestampKind::Absolute, Some(83_004.6), None, None)),
            "@01:23.005"
        );
        assert_eq!(
            timestamp(&ts(TimestampKind::Absolute, Some(-5.0), None, None)),
            "@00:00.000"
        );
        assert_eq!(
            timestamp(&ts(TimestampKind::MeasureBeat, None, Some(3), Some(2.5))),
            "@3/2.5"
        );
        assert_eq!(
            timestamp(&ts(TimestampKind::MeasureBeat, None, None, None)),
            "@1/1"
        );
    }

    #[test]
    fn a_comment_between_a_lists_elements_is_written_back_in_place() {
        let cases = [
            (
                "tempo {\n    bpm: 100\n    changes: [\n        @2/1 { bpm: 120, # c\n            transition: 2m }\n    ]\n}\nshow \"S\" {\n    @1/1\n    a: static, color: \"red\", duration: 1s\n}\n",
                "@2/1 { bpm: 120, # c",
            ),
            (
                "sequence \"x\"(\n    hit, # c\n    bed // d\n), length: 1measure {\n    @1/1\n    a: static, color: $hit, duration: 1beat\n}\n",
                "sequence \"x\"(hit, # c",
            ),
            (
                "show \"S\" {\n    @1/1\n    movers: move, pan: 45deg, # c\n        tilt: 10deg, duration: 1s\n}\n",
                "movers: move, pan: 45deg, # c",
            ),
            (
                "show \"S\" {\n    @1/1\n    master(layer: background, # c\n        intensity: 50%)\n}\n",
                "master(layer: background, # c",
            ),
            (
                "show \"S\" {\n    @1/1\n    sequence \"x\"(hit: \"#FF0000\", # c\n        bed: \"#0000FF\")\n}\n",
                "sequence \"x\"(hit: \"#FF0000\", # c",
            ),
            (
                "show \"S\" {\n    @1/1\n    front, # c\n    back: static, color: \"red\", duration: 1s\n}\n",
                "front, # c",
            ),
        ];
        for (source, on_line) in cases {
            let doc = read(source);
            let written = write(&doc);
            assert_eq!(unlined(&read(&written)), unlined(&doc), "{written}");
            assert_eq!(comment_lines(&written), comment_lines(source));
            assert!(written.lines().any(|l| l.trim() == on_line), "{written}");
        }
    }

    #[test]
    fn a_tempo_start_is_written_as_read_until_its_value_or_unit_changes() {
        let doc = read("tempo {\n    start: 500ms\n    bpm: 120\n}\nshow \"S\" {\n    @1/1\n}\n");
        let with_start = |value: f64, unit: DurationUnit| {
            let mut doc = doc.clone();
            let tempo = doc.tempo.as_mut().unwrap();
            tempo.start = DurationValue {
                value,
                unit,
                written: tempo.start.written.clone(),
            };
            write(&doc)
        };
        assert!(write(&doc).contains("start: 500ms\n"));
        // `500ms` ends with `s`: switched to seconds it is no longer that text.
        assert!(with_start(500.0, DurationUnit::S).contains("start: 500s\n"));
        assert!(with_start(250.0, DurationUnit::Ms).contains("start: 250ms\n"));
        assert!(with_start(0.5, DurationUnit::S).contains("start: 0.5s\n"));
        assert!(write(&read("tempo {\n    start: 0.50s\n}\n")).contains("start: 0.50s\n"));
        let w = WrittenDuration {
            text: "0.50s".to_string(),
            value: 0.5,
            unit: DurationUnit::S,
        };
        assert_eq!(
            read("tempo {\n    start: 0.50s\n}\n")
                .tempo
                .unwrap()
                .start
                .written,
            Some(w)
        );
    }

    #[test]
    fn a_tempo_field_left_out_is_written_only_once_set() {
        let doc = read("tempo {\n    bpm: 100\n}\n");
        assert_eq!(write(&doc), "tempo {\n    bpm: 100\n}\n");
        let mut edited = doc.clone();
        let tempo = edited.tempo.as_mut().unwrap();
        tempo.time_signature = (3, 4);
        tempo.bpm = 120.5;
        assert_eq!(
            write(&edited),
            "tempo {\n    bpm: 120.5\n    time_signature: 3/4\n}\n"
        );
    }

    #[test]
    fn each_offset_line_keeps_its_own_comments() {
        let source = show("    offset 1 measures # a\n    # between\n    offset 2 measures # b\n");
        assert_eq!(write(&read(&source)), source);
        // An edited sum is one line, every comment kept in order.
        let edited = edit_cue(&source, |cue| cue.offset_measures = Some(5));
        assert_eq!(
            edited,
            show("    # a\n    # between\n    offset 5 measures # b\n")
        );
    }

    #[test]
    fn an_edit_changes_only_the_edited_parameters_text() {
        let source = "tempo {\n    bpm: 100\n    changes: [\n        @2/1 { transition: 2m, bpm: 120.0 } # last\n    ]\n}\nshow \"S\" {\n    @1/1\n    master(intensity: 50%, # c\n        layer: background)\n}\n";
        let mut doc = read(source);
        let tempo = doc.tempo.as_mut().unwrap();
        tempo.changes[0].bpm = Some(140.0);
        // A change added after a last one with a comment after its `}`.
        tempo.changes.push(TempoChange {
            timestamp: Timestamp {
                kind: TimestampKind::MeasureBeat,
                text: None,
                ms: None,
                measure: Some(9),
                beat: Some(1.0),
            },
            bpm: Some(90.0),
            time_signature: None,
            transition: None,
            written: None,
            comment_before_comma: None,
            comments_before: None,
            comment: None,
        });
        let cmd = &mut doc.shows[0].cues[0].commands[0];
        cmd.intensity = Some("80%".to_string());
        cmd.speed = Some("2".to_string());
        let written = write(&doc);
        let lines: Vec<&str> = written.lines().map(str::trim).collect();
        assert!(
            lines.contains(&"@2/1 { transition: 2m, bpm: 140 }, # last"),
            "{written}"
        );
        assert!(lines.contains(&"@9/1 { bpm: 90 }"), "{written}");
        assert!(!lines.contains(&","), "{written}");
        assert!(lines.contains(&"master(intensity: 80%, # c"), "{written}");
        assert!(lines.contains(&"layer: background, speed: 2)"), "{written}");
        assert!(document(&written, None).error.is_none());

        // An effect's parameter set by a form keeps the comment after it.
        let panned = edit_effect(
            &show("    movers: move, pan: 45deg, # c\n        tilt: 10deg, duration: 1s\n"),
            set("pan", Some("50deg")),
        );
        assert!(
            panned.contains("movers: move, pan: 50deg, # c\n"),
            "{panned}"
        );
    }

    #[test]
    fn a_last_parameters_comment_and_the_effects_stay_two_comments() {
        let panned = edit_effect(
            &show("    movers: move, pan: 45deg, # c\n        tilt: 10deg # t\n"),
            set("tilt", Some("")),
        );
        assert_eq!(panned, show("    movers: move, pan: 45deg # c\n    # t\n"));
        let coloured = edit_effect(
            &show("    all: static, color: \"red\", # first\n        color: \"blue\" # second\n"),
            EffectEdit::Colors {
                colors: vec!["green".to_string()],
            },
        );
        assert_eq!(
            coloured,
            show("    all: static, color: \"green\" # first\n    # second\n")
        );
    }

    #[test]
    fn a_comment_after_a_references_arguments_stays_before_its_loop() {
        let source = show("    sequence \"x\"(hit: \"#FF0000\"), # c\n        loop: 2\n    sequence \"y\" # e\n        # above\n        loop: 3\n");
        assert_eq!(write(&read(&source)), source);
    }

    #[test]
    fn an_element_removed_leaves_its_comments_where_it_was() {
        let master = edit_cue(
            &show("    master(layer: background, # c\n        intensity: 50%)\n"),
            |cue| cue.commands[0].layer = None,
        );
        assert_eq!(
            master,
            show("    master(\n        # c\n        intensity: 50%)\n")
        );
        let panned = edit_effect(
            &show("    movers: move, pan: 45deg, # c\n        tilt: 10deg, duration: 1s\n"),
            set("pan", None),
        );
        assert_eq!(
            panned,
            show("    movers: move,\n        # c\n        tilt: 10deg, duration: 1s\n")
        );
        let offset = edit_cue(
            &show("    offset 1 measures # a\n    a: static, color: \"red\", duration: 1s\n"),
            |cue| cue.offset_measures = None,
        );
        assert_eq!(
            offset,
            show("    # a\n    a: static, color: \"red\", duration: 1s\n")
        );
        for written in [master, panned, offset] {
            assert!(document(&written, None).error.is_none(), "{written}");
        }
    }

    #[test]
    fn an_effects_parameters_all_removed_leave_their_comments_after_it() {
        let cleared = edit_effect(
            &show("    # above\n    front: static, color: \"red\", # one\n        # two\n        dimmer: 50%\n    clear(layer: background)\n"),
            EffectEdit::Remove {
                names: vec!["color".to_string(), "dimmer".to_string()],
            },
        );
        assert_eq!(
            cleared,
            show("    # above\n    front: static\n    # one\n    # two\n    clear(layer: background)\n")
        );
        assert!(document(&cleared, None).error.is_none());
    }

    #[test]
    fn items_added_or_removed_are_written_grouped() {
        let mut doc = document(
            "show \"A\" {\n    @1/1\n}\n\ninclude \"x.light\"\n",
            Some(Path::new("show.light")),
        )
        .document
        .unwrap();
        assert_eq!(
            write(&doc),
            "show \"A\" {\n    @1/1\n}\n\ninclude \"x.light\"\n"
        );
        doc.shows.clear();
        assert_eq!(write(&doc), "include \"x.light\"\n");
    }

    #[test]
    fn a_length_is_named_in_bars_beats_or_time() {
        assert_eq!(duration_text(Some((1.0, 4.0)), 2000.0), "1measure");
        assert_eq!(duration_text(Some((2.0004, 8.0)), 4000.0), "2measures");
        assert_eq!(duration_text(Some((0.25, 1.0)), 500.0), "1beat");
        assert_eq!(duration_text(Some((0.5, 2.0)), 1000.0), "2beats");
        assert_eq!(duration_text(Some((0.4, 1.5)), 750.0), "750ms");
        assert_eq!(duration_text(None, 5000.0), "5s");
        assert_eq!(duration_text(None, 249.6), "250ms");
        assert_eq!(duration_text(None, 2500.0), "2500ms");
    }

    #[test]
    fn a_transition_is_written_as_the_grammar_reads_it() {
        use super::super::super::tempo::TransitionCurve;
        for (t, text) in [
            (TempoTransition::Beats(4.0, TransitionCurve::Linear), "4"),
            (TempoTransition::Beats(2.5, TransitionCurve::Linear), "2.5"),
            (
                TempoTransition::Measures(2.0, TransitionCurve::Linear),
                "2m",
            ),
            (TempoTransition::Snap, "snap"),
        ] {
            assert_eq!(transition_text(&t), text);
            assert_eq!(super::super::document::transition(text), Some(t));
        }
        assert_eq!(super::super::document::transition("2 m"), None);
        assert_eq!(super::super::document::transition("2m x"), None);
    }

    #[test]
    fn numbers_are_written_as_the_editor_always_wrote_them() {
        for (n, text) in [
            (120.0, "120"),
            (120.5, "120.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (-0.0, "0"),
            (1e21, "1e+21"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
        ] {
            assert_eq!(number(n), text);
        }
    }
}
