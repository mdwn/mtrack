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
//! The editor's edits of a file's `palette { }` block: add a slot, set its
//! colour, rename it (every `palette.<slot>` the file writes follows) or
//! remove it (refused while something uses it, with the uses listed). Each
//! edit is a [`PaletteEdit`] on a [`Document`], so the editor holds the
//! file and changes it without writing the grammar itself, like
//! [`super::edit`]'s effect edits.

use pest::Parser;
use serde::{Deserialize, Serialize};

use super::document::{
    Cue, Document, DocumentItem, PaletteSection, PaletteSlot, Remarks, Timestamp, TimestampKind,
};
use super::edit::quoted;
use super::grammar::{LightingParser, Rule};
use super::palette::{slot_of, REFERENCE_PREFIX};
use super::utils::parse_color_string;

/// An edit of a file's palette, as the editor makes it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PaletteEdit {
    /// A new slot with its default colour (as typed: `#c01020`, `orange`,
    /// `rgb(1, 2, 3)`), at the end of the block, which is made when the
    /// file has none.
    Add { name: String, value: String },
    /// A slot's default colour changed.
    Set { name: String, value: String },
    /// A slot renamed, and every reference to it in the file with it.
    Rename { from: String, to: String },
    /// A slot removed, when nothing uses it; the block with its last slot.
    Remove { name: String },
}

/// What an edit of a palette gives: the document with it made, or why not.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PaletteEditResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub document: Option<Document>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub error: Option<PaletteEditError>,
}

/// Why a palette edit was refused.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PaletteEditError {
    pub code: PaletteEditCode,
    /// The slot the refusal is about: the one named, or for `exists` the
    /// name already taken.
    pub slot: String,
    /// For `in_use`, where the slot is referred to.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub uses: Vec<PaletteUse>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaletteEditCode {
    /// The name is not one the grammar reads as an identifier.
    BadName,
    /// The colour is none the parser reads.
    BadColor,
    /// The file has a slot of that name already.
    Exists,
    /// The file has no slot of that name.
    Unknown,
    /// The slot is referred to, so it stays.
    InUse,
}

/// One place a slot is referred to.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PaletteUse {
    pub container: PaletteContainer,
    /// The show's or sequence's name (`""` for an unnamed show).
    pub name: String,
    /// The cue it is in, as its `@` line reads; none for a sequence's
    /// parameter default.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub at: Option<String>,
    /// The parameter or argument that holds it (`color`, `accent`), or
    /// `loop` for one inside an inline loop's text.
    pub param: String,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaletteContainer {
    Show,
    Sequence,
}

impl Document {
    /// The document with `edit` made, or why it is refused. Only the text
    /// the edit touches changes: the block keeps its comments, a slot its
    /// place.
    pub fn palette_edited(&self, edit: &PaletteEdit) -> PaletteEditResult {
        match self.try_palette_edit(edit) {
            Ok(document) => PaletteEditResult {
                document: Some(document),
                error: None,
            },
            Err(error) => PaletteEditResult {
                document: None,
                error: Some(error),
            },
        }
    }

    fn try_palette_edit(&self, edit: &PaletteEdit) -> Result<Document, PaletteEditError> {
        let refuse = |code, slot: &str| PaletteEditError {
            code,
            slot: slot.to_string(),
            uses: Vec::new(),
        };
        let mut next = self.clone();
        match edit {
            PaletteEdit::Add { name, value } => {
                if !is_identifier(name) {
                    return Err(refuse(PaletteEditCode::BadName, name));
                }
                let written =
                    written_color(value).ok_or_else(|| refuse(PaletteEditCode::BadColor, name))?;
                if self.slot(name).is_some() {
                    return Err(refuse(PaletteEditCode::Exists, name));
                }
                let slot = PaletteSlot {
                    name: name.clone(),
                    value: written,
                    remarks: Remarks::default(),
                };
                match next.palette.as_mut() {
                    Some(section) => section.slots.push(slot),
                    None => {
                        next.palette = Some(PaletteSection {
                            slots: vec![slot],
                            comments_before: None,
                            comment: None,
                            comments_after: None,
                            close_comment: None,
                        });
                        next.place_palette_item();
                    }
                }
            }
            PaletteEdit::Set { name, value } => {
                let written =
                    written_color(value).ok_or_else(|| refuse(PaletteEditCode::BadColor, name))?;
                let slot = next
                    .palette
                    .as_mut()
                    .and_then(|p| p.slots.iter_mut().find(|s| &s.name == name))
                    .ok_or_else(|| refuse(PaletteEditCode::Unknown, name))?;
                slot.value = written;
            }
            PaletteEdit::Rename { from, to } => {
                if self.slot(from).is_none() {
                    return Err(refuse(PaletteEditCode::Unknown, from));
                }
                if from == to {
                    return Ok(next);
                }
                if !is_identifier(to) {
                    return Err(refuse(PaletteEditCode::BadName, to));
                }
                if self.slot(to).is_some() {
                    return Err(refuse(PaletteEditCode::Exists, to));
                }
                if let Some(slot) = next
                    .palette
                    .as_mut()
                    .and_then(|p| p.slots.iter_mut().find(|s| &s.name == from))
                {
                    slot.name = to.clone();
                }
                next.each_value(&mut |_, text| {
                    if let Some(renamed) = renamed_reference(text, from, to) {
                        *text = renamed;
                    }
                });
            }
            PaletteEdit::Remove { name } => {
                if self.slot(name).is_none() {
                    return Err(refuse(PaletteEditCode::Unknown, name));
                }
                let uses = self.palette_uses(name);
                if !uses.is_empty() {
                    return Err(PaletteEditError {
                        code: PaletteEditCode::InUse,
                        slot: name.clone(),
                        uses,
                    });
                }
                if let Some(section) = next.palette.as_mut() {
                    section.slots.retain(|s| &s.name != name);
                    if section.slots.is_empty() {
                        next.palette = None;
                        if let Some(items) = next.items.as_mut() {
                            items.retain(|i| *i != DocumentItem::Palette);
                        }
                    }
                }
            }
        }
        Ok(next)
    }

    fn slot(&self, name: &str) -> Option<&PaletteSlot> {
        self.palette
            .as_ref()
            .and_then(|p| p.slots.iter().find(|s| s.name == name))
    }

    /// Where the file refers to `slot` as `palette.<slot>`, in the order
    /// it is written: sequences before shows, as the document holds them.
    pub fn palette_uses(&self, slot: &str) -> Vec<PaletteUse> {
        let mut uses = Vec::new();
        // The walk is over a copy: it is the one that can change values.
        self.clone().each_value(&mut |place, text| {
            if refers_to(text, slot) {
                uses.push(place);
            }
        });
        uses
    }

    /// The palette's place among the items, when the file wrote its items
    /// in an order: after the leading includes and tempo, where the
    /// writer puts a block of a file that never had one.
    fn place_palette_item(&mut self) {
        if let Some(items) = self.items.as_mut() {
            let at = items
                .iter()
                .take_while(|i| matches!(i, DocumentItem::Include | DocumentItem::Tempo))
                .count();
            items.insert(at, DocumentItem::Palette);
        }
    }

    /// Calls `f` with every text in the file that can hold a reference to
    /// a slot -- an effect's parameter, a sequence's argument or default,
    /// an inline loop's text -- and where it is.
    fn each_value(&mut self, f: &mut dyn FnMut(PaletteUse, &mut String)) {
        for sequence in &mut self.sequences {
            for param in sequence.params.iter_mut().flatten() {
                if let Some(value) = param.value.as_mut() {
                    let place = PaletteUse {
                        container: PaletteContainer::Sequence,
                        name: sequence.name.clone(),
                        at: None,
                        param: param.name.clone(),
                    };
                    f(place, value);
                }
            }
            each_cue_value(
                &mut sequence.cues,
                PaletteContainer::Sequence,
                &sequence.name,
                f,
            );
        }
        for show in &mut self.shows {
            each_cue_value(&mut show.cues, PaletteContainer::Show, &show.name, f);
        }
    }
}

fn each_cue_value(
    cues: &mut [Cue],
    container: PaletteContainer,
    name: &str,
    f: &mut dyn FnMut(PaletteUse, &mut String),
) {
    for cue in cues {
        let at = Some(timestamp_text(&cue.timestamp));
        let place = |param: &str| PaletteUse {
            container,
            name: name.to_string(),
            at: at.clone(),
            param: param.to_string(),
        };
        for effect in &mut cue.effects {
            for param in &mut effect.effect.params {
                f(place(&param.name), &mut param.value);
            }
        }
        for reference in &mut cue.sequences {
            for arg in reference.args.iter_mut().flatten() {
                if let Some(value) = arg.value.as_mut() {
                    f(place(&arg.name), value);
                }
            }
        }
        for inline in cue.loops.iter_mut().flatten() {
            f(place("loop"), &mut inline.text);
        }
    }
}

/// A cue's position as its line writes it.
fn timestamp_text(ts: &Timestamp) -> String {
    if let Some(text) = ts.text.as_deref().filter(|t| !t.is_empty()) {
        return text.to_string();
    }
    match ts.kind {
        TimestampKind::MeasureBeat => {
            format!("@{}/{}", ts.measure.unwrap_or(1), ts.beat.unwrap_or(1.0))
        }
        TimestampKind::Absolute => format!("@{}ms", ts.ms.unwrap_or(0.0)),
    }
}

/// Whether `name` is an identifier by the grammar's own rule.
fn is_identifier(name: &str) -> bool {
    LightingParser::parse(Rule::identifier, name)
        .ok()
        .and_then(|mut pairs| pairs.next())
        .is_some_and(|pair| pair.as_span().end() == name.len())
}

/// A colour as a palette slot writes it, or `None` for text that is none.
fn written_color(value: &str) -> Option<String> {
    let value = value.trim();
    parse_color_string(value)?;
    Some(quoted(value.trim_matches('"')))
}

/// The characters an identifier goes on with.
fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether `text` refers to `slot`: as a whole value, or inside the text
/// of an inline loop.
fn refers_to(text: &str, slot: &str) -> bool {
    slot_of(text) == Some(slot) || !references_in(text, slot).is_empty()
}

/// Where in `text` (byte offsets) `palette.<slot>` is written as a word of
/// its own.
fn references_in(text: &str, slot: &str) -> Vec<usize> {
    let needle = format!("{REFERENCE_PREFIX}{slot}");
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find(&needle) {
        let start = from + at;
        let end = start + needle.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.is_some_and(ident_char) && !after.is_some_and(ident_char) {
            found.push(start);
        }
        from = end;
    }
    found
}

/// `text` with `palette.<from>` written `palette.<to>`, or `None` when it
/// does not refer to `from`.
fn renamed_reference(text: &str, from: &str, to: &str) -> Option<String> {
    let at = references_in(text, from);
    if at.is_empty() {
        return None;
    }
    let old = format!("{REFERENCE_PREFIX}{from}");
    let new = format!("{REFERENCE_PREFIX}{to}");
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for start in at {
        out.push_str(&text[last..start]);
        out.push_str(&new);
        last = start + old.len();
    }
    out.push_str(&text[last..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::document::document;
    use super::super::writer::write;
    use super::*;

    const FILE: &str = r##"tempo {
    start: 0.0s
    bpm: 120.0
    time_signature: 4/4
}

# the look
palette {
    primary: "#c01020"  # the main one
    secondary: "orange"
}

sequence "hit"(color: palette.primary, other: "blue") {
    @1/1
    flash: static color: $color, duration: 1beat
}

show "Main" {
    @1/1
    front: static color: palette.primary, duration: 2measures
    back: cycle color: palette.primary, color: palette.secondary, speed: 1, duration: 4measures
    @3/1
    sequence "hit"(color: palette.secondary)
}
"##;

    fn read(source: &str) -> Document {
        {
            let r = document(source, None);
            r.document.unwrap_or_else(|| panic!("{:?}", r.error))
        }
    }

    fn edit(doc: &Document, edit: PaletteEdit) -> Document {
        let result = doc.palette_edited(&edit);
        assert!(result.error.is_none(), "{:?}", result.error);
        let next = result.document.expect("a document");
        // What was written reads back as the same document.
        assert_eq!(
            read(&write(&next)).palette,
            next.palette,
            "the block round-trips"
        );
        next
    }

    fn refused(doc: &Document, edit: PaletteEdit) -> PaletteEditError {
        doc.palette_edited(&edit).error.expect("refused")
    }

    fn names(doc: &Document) -> Vec<String> {
        doc.palette
            .iter()
            .flat_map(|p| p.slots.iter().map(|s| s.name.clone()))
            .collect()
    }

    #[test]
    fn a_slot_is_added_at_the_end_and_written_quoted() {
        let doc = read(FILE);
        let next = edit(
            &doc,
            PaletteEdit::Add {
                name: "accent".to_string(),
                value: "#00ff80".to_string(),
            },
        );
        assert_eq!(names(&next), ["primary", "secondary", "accent"]);
        let text = write(&next);
        assert!(text.contains("accent: \"#00ff80\""), "{text}");
        // The block keeps its comments.
        assert!(text.contains("# the look") && text.contains("# the main one"));
        assert!(text.contains("accent"), "{text}");
    }

    #[test]
    fn the_first_slot_makes_the_block_after_the_tempo() {
        let source = "tempo {\n    start: 0.0s\n    bpm: 120.0\n    time_signature: 4/4\n}\n\nshow \"S\" {\n    @1/1\n    a: static color: \"red\", duration: 1beat\n}\n";
        let doc = read(source);
        assert!(doc.palette.is_none());
        let next = edit(
            &doc,
            PaletteEdit::Add {
                name: "p".to_string(),
                value: "red".to_string(),
            },
        );
        let text = write(&next);
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle}: {text}"))
        };
        assert!(at("tempo {") < at("palette {"));
        assert!(at("palette {") < at("show"));
        // The player reads it back with the slot.
        assert!(crate::lighting::parser::parse_light_shows(&text).is_ok());
    }

    #[test]
    fn a_name_the_grammar_does_not_read_is_refused() {
        let doc = read(FILE);
        for bad in ["", "1x", "has space", "a.b", "a:b"] {
            let err = refused(
                &doc,
                PaletteEdit::Add {
                    name: bad.to_string(),
                    value: "red".to_string(),
                },
            );
            assert_eq!(err.code, PaletteEditCode::BadName, "{bad:?}");
        }
        // Dashes and underscores are identifier characters.
        edit(
            &doc,
            PaletteEdit::Add {
                name: "back-light_2".to_string(),
                value: "red".to_string(),
            },
        );
    }

    #[test]
    fn a_colour_the_parser_does_not_read_is_refused() {
        let doc = read(FILE);
        let err = refused(
            &doc,
            PaletteEdit::Add {
                name: "x".to_string(),
                value: "notacolor".to_string(),
            },
        );
        assert_eq!(err.code, PaletteEditCode::BadColor);
        let err = refused(
            &doc,
            PaletteEdit::Set {
                name: "primary".to_string(),
                value: "#12".to_string(),
            },
        );
        assert_eq!(err.code, PaletteEditCode::BadColor);
    }

    #[test]
    fn a_name_in_use_or_missing_is_refused() {
        let doc = read(FILE);
        let err = refused(
            &doc,
            PaletteEdit::Add {
                name: "primary".to_string(),
                value: "red".to_string(),
            },
        );
        assert_eq!(err.code, PaletteEditCode::Exists);
        let err = refused(
            &doc,
            PaletteEdit::Set {
                name: "ghost".to_string(),
                value: "red".to_string(),
            },
        );
        assert_eq!(err.code, PaletteEditCode::Unknown);
        let err = refused(
            &doc,
            PaletteEdit::Rename {
                from: "primary".to_string(),
                to: "secondary".to_string(),
            },
        );
        assert_eq!(
            (err.code, err.slot.as_str()),
            (PaletteEditCode::Exists, "secondary")
        );
        let err = refused(
            &doc,
            PaletteEdit::Rename {
                from: "ghost".to_string(),
                to: "x".to_string(),
            },
        );
        assert_eq!(err.code, PaletteEditCode::Unknown);
    }

    #[test]
    fn a_colour_is_set_where_the_slot_is() {
        let doc = read(FILE);
        let next = edit(
            &doc,
            PaletteEdit::Set {
                name: "primary".to_string(),
                value: "rgb(1, 2, 3)".to_string(),
            },
        );
        let text = write(&next);
        assert!(
            text.contains("primary: \"rgb(1, 2, 3)\" # the main one"),
            "{text}"
        );
        assert_eq!(names(&next), ["primary", "secondary"]);
    }

    #[test]
    fn a_rename_rewrites_every_reference_in_the_file() {
        let doc = read(FILE);
        crate::lighting::parser::parse_light_shows(&write(&doc))
            .expect("the file parses as written");
        let next = edit(
            &doc,
            PaletteEdit::Rename {
                from: "primary".to_string(),
                to: "lead".to_string(),
            },
        );
        assert_eq!(names(&next), ["lead", "secondary"]);
        let text = write(&next);
        assert!(!text.contains("palette.primary"), "{text}");
        // A sequence's default, an effect's colour and a cycle's.
        assert_eq!(text.matches("palette.lead").count(), 3, "{text}");
        assert!(text.contains("palette.secondary"), "{text}");
        // The player reads the renamed file, every reference resolving.
        let shows = crate::lighting::parser::parse_light_shows(&text).expect("parses");
        assert!(shows["Main"].palette.contains_key("lead"));
        // And a rename back is the file it was.
        let back = edit(
            &next,
            PaletteEdit::Rename {
                from: "lead".to_string(),
                to: "primary".to_string(),
            },
        );
        assert_eq!(write(&back), write(&doc));
    }

    #[test]
    fn a_rename_leaves_a_longer_name_alone() {
        let source = FILE.replace(
            "secondary: \"orange\"",
            "secondary: \"orange\"\n    primary2: \"blue\"\n    my-primary: \"green\"",
        ) + "\nsequence \"x\" {\n    @1/1\n    a: static color: palette.primary2, duration: 1beat\n    b: static color: palette.my-primary, duration: 1beat\n}\n";
        let doc = read(&source);
        let next = edit(
            &doc,
            PaletteEdit::Rename {
                from: "primary".to_string(),
                to: "lead".to_string(),
            },
        );
        let text = write(&next);
        assert!(text.contains("palette.primary2"), "{text}");
        assert!(text.contains("palette.my-primary"), "{text}");
        assert!(!text.contains("palette.primary,") && !text.contains("palette.primary\n"));
    }

    #[test]
    fn a_slot_in_use_is_not_removed_and_its_uses_are_listed() {
        let doc = read(FILE);
        let err = refused(
            &doc,
            PaletteEdit::Remove {
                name: "primary".to_string(),
            },
        );
        assert_eq!(err.code, PaletteEditCode::InUse);
        let places: Vec<(PaletteContainer, &str, Option<&str>, &str)> = err
            .uses
            .iter()
            .map(|u| {
                (
                    u.container,
                    u.name.as_str(),
                    u.at.as_deref(),
                    u.param.as_str(),
                )
            })
            .collect();
        assert_eq!(
            places,
            vec![
                (PaletteContainer::Sequence, "hit", None, "color"),
                (PaletteContainer::Show, "Main", Some("@1/1"), "color"),
                (PaletteContainer::Show, "Main", Some("@1/1"), "color"),
            ]
        );
    }

    #[test]
    fn an_unused_slot_is_removed_and_the_last_takes_the_block() {
        let source = "palette {\n    a: \"red\"\n    b: \"blue\"\n}\n\nshow \"S\" {\n    @0.000\n    x: static color: palette.a, duration: 1s\n}\n";
        let doc = read(source);
        let next = edit(
            &doc,
            PaletteEdit::Remove {
                name: "b".to_string(),
            },
        );
        assert_eq!(names(&next), ["a"]);
        // Once nothing uses `a` either, the block goes.
        let mut bare = next.clone();
        bare.shows[0].cues[0].effects[0].effect.params[0].value = "\"red\"".to_string();
        let gone = edit(
            &bare,
            PaletteEdit::Remove {
                name: "a".to_string(),
            },
        );
        assert!(gone.palette.is_none());
        let text = write(&gone);
        assert!(!text.contains("palette"), "{text}");
        assert!(crate::lighting::parser::parse_light_shows(&text).is_ok());
    }

    #[test]
    fn a_reference_in_a_loops_text_is_found_and_renamed_as_a_word() {
        let text =
            "side: static color: palette.a, duration: 1beat\n other: palette.ab, x: mypalette.a";
        assert_eq!(references_in(text, "a"), vec![20]);
        assert_eq!(
            renamed_reference(text, "a", "z").as_deref(),
            Some("side: static color: palette.z, duration: 1beat\n other: palette.ab, x: mypalette.a")
        );
        assert_eq!(renamed_reference(text, "q", "z"), None);
    }

    #[test]
    fn a_slot_used_somewhere_has_uses() {
        let doc = read(FILE);
        let used = doc.palette_uses("secondary");
        assert_eq!(used.len(), 2, "{used:?}");
        assert!(doc.palette_uses("ghost").is_empty());
    }
}
