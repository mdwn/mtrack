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
//! A light show's operations: a cue added, moved or taken out, an effect
//! added, changed or taken out. Each works on a `.light` file's
//! [`Document`], which keeps the file as it was written, so that what
//! [`writer::write`] gives back differs from the file only where the
//! operation changed it; and each answers its inverse, which, applied to the
//! result, gives back the document it was applied to, comments and all.
//!
//! An operation names a show by its place among the file's shows, a cue by
//! its place among the show's cues as written, and an effect by its place
//! among the cue's effects. [`Applied::moved`] says how an operation moved
//! the places after it, for whoever keeps its own names for them.
//!
//! What an operation does not do is decide whether the file still plays:
//! [`checked`] says that, by the parser the player loads it with.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::lighting::effects::LayerSet;
use crate::lighting::parser::document::{Cue, CueEffect, CueLine, Document, Timestamp};
use crate::lighting::parser::edit::EffectEdit;
use crate::lighting::parser::writer;

use super::Operation;

/// The light show's operations, by name.
pub const OPERATIONS: &[Operation] = &[
    Operation {
        name: "light_show.insert_cue",
        area: "light_show",
    },
    Operation {
        name: "light_show.remove_cue",
        area: "light_show",
    },
    Operation {
        name: "light_show.set_cue_time",
        area: "light_show",
    },
    Operation {
        name: "light_show.insert_effect",
        area: "light_show",
    },
    Operation {
        name: "light_show.remove_effect",
        area: "light_show",
    },
    Operation {
        name: "light_show.replace_effect",
        area: "light_show",
    },
    Operation {
        name: "light_show.edit_effect",
        area: "light_show",
    },
];

/// One edit of a light show's file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LightShowOp {
    /// `cue` put at place `at` among the show's cues (at the end, when `at`
    /// is their number).
    InsertCue { show: usize, at: usize, cue: Cue },
    /// The cue at place `cue` taken out, with what was written with it.
    RemoveCue { show: usize, cue: usize },
    /// The cue's time made `timestamp`. Its place among the cues as written
    /// stays: the player plays cues by their times.
    SetCueTime {
        show: usize,
        cue: usize,
        timestamp: Timestamp,
    },
    /// `effect` put at place `at` among the cue's effects. `line` is where
    /// among all the cue's lines, as written, it goes: by the inverse of a
    /// removal, which puts it back where it was; without one, after the
    /// effect before it, or first.
    InsertEffect {
        show: usize,
        cue: usize,
        at: usize,
        effect: CueEffect,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
    },
    /// The effect at place `effect` taken out, with its comments.
    RemoveEffect {
        show: usize,
        cue: usize,
        effect: usize,
    },
    /// The effect at place `effect` made `with`, whole.
    ReplaceEffect {
        show: usize,
        cue: usize,
        effect: usize,
        with: CueEffect,
    },
    /// The effect at place `effect` changed by `edit`, as a form changes it
    /// ([`CueEffect::edited`]).
    EditEffect {
        show: usize,
        cue: usize,
        effect: usize,
        edit: EffectEdit,
    },
}

impl LightShowOp {
    /// Its name in [`OPERATIONS`].
    pub fn name(&self) -> &'static str {
        match self {
            LightShowOp::InsertCue { .. } => "light_show.insert_cue",
            LightShowOp::RemoveCue { .. } => "light_show.remove_cue",
            LightShowOp::SetCueTime { .. } => "light_show.set_cue_time",
            LightShowOp::InsertEffect { .. } => "light_show.insert_effect",
            LightShowOp::RemoveEffect { .. } => "light_show.remove_effect",
            LightShowOp::ReplaceEffect { .. } => "light_show.replace_effect",
            LightShowOp::EditEffect { .. } => "light_show.edit_effect",
        }
    }
}

/// How an operation moved the places after the one it changed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "moved", rename_all = "snake_case")]
pub enum Moved {
    /// No place moved.
    Nothing,
    /// A cue was put at `at`: the show's cues from `at` on are one later.
    CueInserted { show: usize, at: usize },
    /// The cue at `at` was taken out: those after it are one earlier.
    CueRemoved { show: usize, at: usize },
    /// An effect was put at `at` in the cue: those from `at` on are one later.
    EffectInserted { show: usize, cue: usize, at: usize },
    /// The effect at `at` was taken out: those after it are one earlier.
    EffectRemoved { show: usize, cue: usize, at: usize },
}

/// What applying an operation did.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Applied {
    /// The operation that takes it back, applied to the document as it now is.
    pub inverse: LightShowOp,
    pub moved: Moved,
}

/// Why an operation could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpError {
    #[error("the file has no show {0}")]
    NoShow(usize),
    #[error("the show has no cue {0}")]
    NoCue(usize),
    #[error("the cue has no effect {0}")]
    NoEffect(usize),
    #[error("a cue cannot go at {0}: the show has fewer cues than that")]
    CueOutOfRange(usize),
    #[error("an effect cannot go at {0}: the cue has fewer effects than that")]
    EffectOutOfRange(usize),
}

/// Applies `op` to `document`, answering its inverse and what it moved. A
/// document `op` cannot be applied to is left as it was.
pub fn apply(document: &mut Document, op: &LightShowOp) -> Result<Applied, OpError> {
    match op.clone() {
        LightShowOp::InsertCue { show, at, cue } => {
            let cues = &mut show_mut(document, show)?.cues;
            if at > cues.len() {
                return Err(OpError::CueOutOfRange(at));
            }
            cues.insert(at, cue);
            Ok(Applied {
                inverse: LightShowOp::RemoveCue { show, cue: at },
                moved: Moved::CueInserted { show, at },
            })
        }
        LightShowOp::RemoveCue { show, cue } => {
            let cues = &mut show_mut(document, show)?.cues;
            if cue >= cues.len() {
                return Err(OpError::NoCue(cue));
            }
            let gone = cues.remove(cue);
            Ok(Applied {
                inverse: LightShowOp::InsertCue {
                    show,
                    at: cue,
                    cue: gone,
                },
                moved: Moved::CueRemoved { show, at: cue },
            })
        }
        LightShowOp::SetCueTime {
            show,
            cue,
            timestamp,
        } => {
            let found = cue_mut(document, show, cue)?;
            let was = std::mem::replace(&mut found.timestamp, timestamp);
            Ok(Applied {
                inverse: LightShowOp::SetCueTime {
                    show,
                    cue,
                    timestamp: was,
                },
                moved: Moved::Nothing,
            })
        }
        LightShowOp::InsertEffect {
            show,
            cue,
            at,
            effect,
            line,
        } => {
            let found = cue_mut(document, show, cue)?;
            if at > found.effects.len() {
                return Err(OpError::EffectOutOfRange(at));
            }
            if let Some(lines) = found.lines.as_mut() {
                let place = line.filter(|l| *l <= lines.len()).unwrap_or_else(|| {
                    // Just after the effect before it; the first, just
                    // before the effect that was first, or first of all.
                    match at.checked_sub(1) {
                        Some(before) => nth_effect_line(lines, before).map(|l| l + 1),
                        None => nth_effect_line(lines, 0),
                    }
                    .unwrap_or(0)
                });
                lines.insert(place, CueLine::Effect);
            }
            found.effects.insert(at, effect);
            Ok(Applied {
                inverse: LightShowOp::RemoveEffect {
                    show,
                    cue,
                    effect: at,
                },
                moved: Moved::EffectInserted { show, cue, at },
            })
        }
        LightShowOp::RemoveEffect { show, cue, effect } => {
            let found = cue_mut(document, show, cue)?;
            if effect >= found.effects.len() {
                return Err(OpError::NoEffect(effect));
            }
            let line = found
                .lines
                .as_mut()
                .and_then(|lines| nth_effect_line(lines, effect).map(|l| (lines, l)))
                .map(|(lines, l)| {
                    lines.remove(l);
                    l
                });
            let gone = found.effects.remove(effect);
            Ok(Applied {
                inverse: LightShowOp::InsertEffect {
                    show,
                    cue,
                    at: effect,
                    effect: gone,
                    line,
                },
                moved: Moved::EffectRemoved {
                    show,
                    cue,
                    at: effect,
                },
            })
        }
        LightShowOp::ReplaceEffect {
            show,
            cue,
            effect,
            with,
        } => {
            let found = effect_mut(document, show, cue, effect)?;
            let was = std::mem::replace(found, with);
            Ok(Applied {
                inverse: LightShowOp::ReplaceEffect {
                    show,
                    cue,
                    effect,
                    with: was,
                },
                moved: Moved::Nothing,
            })
        }
        LightShowOp::EditEffect {
            show,
            cue,
            effect,
            edit,
        } => {
            let found = effect_mut(document, show, cue, effect)?;
            let next = found.edited(&edit);
            let was = std::mem::replace(found, next);
            Ok(Applied {
                inverse: LightShowOp::ReplaceEffect {
                    show,
                    cue,
                    effect,
                    with: was,
                },
                moved: Moved::Nothing,
            })
        }
    }
}

/// A new cue at `timestamp`, with nothing under it.
pub fn new_cue(timestamp: Timestamp) -> Cue {
    Cue::at(timestamp)
}

/// `document` written as its file, when the player would load that file:
/// parsed by the parser the player loads shows with, against `layers` and
/// the song's tempo map `tempo`, its `include`s resolved beside `path`.
/// What the parser says when it would not.
pub fn checked(
    document: &Document,
    layers: &LayerSet,
    path: Option<&Path>,
    tempo: Option<&crate::tempo::TempoMap>,
) -> Result<String, String> {
    let text = writer::write(document);
    crate::lighting::parser::parse_light_file(layers, &text, path, tempo)
        .map(|_| text)
        .map_err(|e| e.to_string())
}

/// Where among a cue's lines, as written, its `n`th effect is.
fn nth_effect_line(lines: &[CueLine], n: usize) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == CueLine::Effect)
        .nth(n)
        .map(|(at, _)| at)
}

fn show_mut(
    document: &mut Document,
    show: usize,
) -> Result<&mut crate::lighting::parser::document::LightShow, OpError> {
    document.shows.get_mut(show).ok_or(OpError::NoShow(show))
}

fn cue_mut(document: &mut Document, show: usize, cue: usize) -> Result<&mut Cue, OpError> {
    show_mut(document, show)?
        .cues
        .get_mut(cue)
        .ok_or(OpError::NoCue(cue))
}

fn effect_mut(
    document: &mut Document,
    show: usize,
    cue: usize,
    effect: usize,
) -> Result<&mut CueEffect, OpError> {
    cue_mut(document, show, cue)?
        .effects
        .get_mut(effect)
        .ok_or(OpError::NoEffect(effect))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::document::{document, TimestampKind};
    use crate::lighting::parser::edit::new_effect;

    /// Every `.light` file in the repository's examples, by path.
    fn examples() -> Vec<(std::path::PathBuf, String)> {
        fn walk(dir: &Path, into: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, into);
                } else if path.extension().is_some_and(|e| e == "light") {
                    into.push(path);
                }
            }
        }
        let mut found = Vec::new();
        walk(Path::new("../../examples"), &mut found);
        found.sort();
        assert!(!found.is_empty(), "the examples have light shows");
        found
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap();
                (path, text)
            })
            .collect()
    }

    fn read(text: &str) -> Document {
        let result = document(text, None);
        result
            .document
            .unwrap_or_else(|| panic!("reads: {:?}", result.error))
    }

    fn at_ms(ms: f64) -> Timestamp {
        Timestamp {
            kind: TimestampKind::Absolute,
            text: None,
            ms: Some(ms),
            measure: None,
            beat: None,
        }
    }

    /// `op` applied to `before` and then taken back writes what `before` does.
    fn comes_back(before: &Document, op: LightShowOp, at: &Path) {
        let written = writer::write(before);
        let mut after = before.clone();
        let applied = apply(&mut after, &op).unwrap_or_else(|e| panic!("{at:?} {op:?}: {e}"));
        let changed = writer::write(&after);
        // What was written reads again.
        read(&changed);
        apply(&mut after, &applied.inverse).unwrap();
        assert_eq!(writer::write(&after), written, "{at:?} {op:?}");
        assert_eq!(&after, before, "{at:?} {op:?}");
    }

    #[test]
    fn every_operation_on_every_example_is_taken_back_text_for_text() {
        let (mut tried, mut shows) = (0, 0);
        for (path, text) in examples() {
            // Fixture types and venues are files of their own kinds.
            let Some(doc) = document(&text, None).document else {
                continue;
            };
            shows += doc.shows.len();
            for (s, show) in doc.shows.iter().enumerate() {
                for at in [0, show.cues.len()] {
                    comes_back(
                        &doc,
                        LightShowOp::InsertCue {
                            show: s,
                            at,
                            cue: new_cue(at_ms(1234.0)),
                        },
                        &path,
                    );
                }
                for (c, cue) in show.cues.iter().enumerate() {
                    comes_back(&doc, LightShowOp::RemoveCue { show: s, cue: c }, &path);
                    comes_back(
                        &doc,
                        LightShowOp::SetCueTime {
                            show: s,
                            cue: c,
                            timestamp: at_ms(4321.0),
                        },
                        &path,
                    );
                    for at in [0, cue.effects.len()] {
                        comes_back(
                            &doc,
                            LightShowOp::InsertEffect {
                                show: s,
                                cue: c,
                                at,
                                effect: new_effect(None, false),
                                line: None,
                            },
                            &path,
                        );
                    }
                    for e in 0..cue.effects.len() {
                        comes_back(
                            &doc,
                            LightShowOp::RemoveEffect {
                                show: s,
                                cue: c,
                                effect: e,
                            },
                            &path,
                        );
                        comes_back(
                            &doc,
                            LightShowOp::EditEffect {
                                show: s,
                                cue: c,
                                effect: e,
                                edit: EffectEdit::Set {
                                    name: "duration".into(),
                                    value: Some("2s".into()),
                                },
                            },
                            &path,
                        );
                        comes_back(
                            &doc,
                            LightShowOp::ReplaceEffect {
                                show: s,
                                cue: c,
                                effect: e,
                                with: new_effect(None, true),
                            },
                            &path,
                        );
                        tried += 3;
                    }
                    tried += 4;
                }
            }
        }
        assert!(shows > 5, "only {shows} shows were read");
        assert!(tried > 100, "only {tried} operations were tried");
    }

    const SHOW: &str = r#"show "Test" {
    # the intro
    @00:00.000
    front_wash: static color: "blue", duration: 5s
    clear(layer: background) // clear it
    back_wash: static color: "red", duration: 5s

    @00:05.000
    front_wash: static color: "green", duration: 5s
}
"#;

    #[test]
    fn an_edit_changes_only_its_own_line_and_comments_stay() {
        let doc = read(SHOW);
        let mut edited = doc.clone();
        apply(
            &mut edited,
            &LightShowOp::SetCueTime {
                show: 0,
                cue: 1,
                timestamp: at_ms(7500.0),
            },
        )
        .unwrap();
        let (was, now) = (writer::write(&doc), writer::write(&edited));
        let changed: Vec<(&str, &str)> = was
            .lines()
            .zip(now.lines())
            .filter(|(a, b)| a != b)
            .collect();
        assert_eq!(changed.len(), 1, "{was}\n---\n{now}");
        assert!(changed[0].1.contains("07.500"), "{now}");
        assert!(
            now.contains("# the intro") && now.contains("// clear it"),
            "{now}"
        );
    }

    #[test]
    fn an_effect_taken_out_comes_back_between_the_lines_it_was_between() {
        let doc = read(SHOW);
        let mut edited = doc.clone();
        let applied = apply(
            &mut edited,
            &LightShowOp::RemoveEffect {
                show: 0,
                cue: 0,
                effect: 0,
            },
        )
        .unwrap();
        assert!(!writer::write(&edited).contains(r#""blue""#));
        assert_eq!(
            applied.moved,
            Moved::EffectRemoved {
                show: 0,
                cue: 0,
                at: 0
            }
        );
        apply(&mut edited, &applied.inverse).unwrap();
        // Above the layer command, as written, not after it.
        let now = writer::write(&edited);
        assert!(now.find(r#""blue""#) < now.find("clear("), "{now}");
        assert_eq!(now, writer::write(&doc));
    }

    #[test]
    fn a_new_effect_goes_after_the_effect_before_it() {
        let mut doc = read(SHOW);
        apply(
            &mut doc,
            &LightShowOp::InsertEffect {
                show: 0,
                cue: 0,
                at: 1,
                effect: new_effect(None, false),
                line: None,
            },
        )
        .unwrap();
        let now = writer::write(&doc);
        let (blue, new, clear) = (
            now.find(r#""blue""#).unwrap(),
            now.find("all: static").unwrap(),
            now.find("clear(").unwrap(),
        );
        assert!(blue < new && new < clear, "{now}");
    }

    #[test]
    fn what_is_not_there_is_refused_and_the_document_kept() {
        let doc = read(SHOW);
        let mut edited = doc.clone();
        for (op, why) in [
            (
                LightShowOp::RemoveCue { show: 3, cue: 0 },
                OpError::NoShow(3),
            ),
            (
                LightShowOp::RemoveCue { show: 0, cue: 9 },
                OpError::NoCue(9),
            ),
            (
                LightShowOp::RemoveEffect {
                    show: 0,
                    cue: 1,
                    effect: 4,
                },
                OpError::NoEffect(4),
            ),
            (
                LightShowOp::InsertCue {
                    show: 0,
                    at: 3,
                    cue: new_cue(at_ms(0.0)),
                },
                OpError::CueOutOfRange(3),
            ),
        ] {
            assert_eq!(apply(&mut edited, &op), Err(why));
        }
        assert_eq!(edited, doc);
    }

    #[test]
    fn checked_says_what_the_player_would_say() {
        let layers = LayerSet::standard();
        let doc = read(SHOW);
        assert!(checked(&doc, layers, None, None).is_ok());
        let mut bad = doc.clone();
        apply(
            &mut bad,
            &LightShowOp::EditEffect {
                show: 0,
                cue: 0,
                effect: 0,
                edit: EffectEdit::Set {
                    name: "layer".into(),
                    value: Some("no_such_layer".into()),
                },
            },
        )
        .unwrap();
        assert!(checked(&bad, layers, None, None).is_err());
    }

    #[test]
    fn every_operation_is_listed_by_the_name_it_has() {
        let named: Vec<&str> = OPERATIONS.iter().map(|o| o.name).collect();
        let ts = at_ms(0.0);
        let effect = new_effect(None, false);
        for op in [
            LightShowOp::InsertCue {
                show: 0,
                at: 0,
                cue: new_cue(ts.clone()),
            },
            LightShowOp::RemoveCue { show: 0, cue: 0 },
            LightShowOp::SetCueTime {
                show: 0,
                cue: 0,
                timestamp: ts,
            },
            LightShowOp::InsertEffect {
                show: 0,
                cue: 0,
                at: 0,
                effect: effect.clone(),
                line: None,
            },
            LightShowOp::RemoveEffect {
                show: 0,
                cue: 0,
                effect: 0,
            },
            LightShowOp::ReplaceEffect {
                show: 0,
                cue: 0,
                effect: 0,
                with: effect,
            },
            LightShowOp::EditEffect {
                show: 0,
                cue: 0,
                effect: 0,
                edit: EffectEdit::Remove { names: vec![] },
            },
        ] {
            assert!(named.contains(&op.name()), "{}", op.name());
            // And it says so as data, as it is asked for.
            let json = serde_json::to_value(&op).unwrap();
            assert_eq!(serde_json::from_value::<LightShowOp>(json).unwrap(), op);
        }
        assert_eq!(named.len(), 7);
        assert!(super::super::OPERATIONS
            .iter()
            .all(|o| o.name.starts_with(o.area)));
    }
}
