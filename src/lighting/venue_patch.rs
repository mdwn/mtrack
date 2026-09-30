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

//! The one way a venue file is edited by the web UI: a **textual patch** of
//! the existing file, not a regeneration. Regenerating a venue from its
//! parsed form loses everything the parser skips — a header comment, the
//! `# TODO fixture ...` lines an MVR import writes for fixtures it could not
//! resolve, trailing `# layer "..."` notes — so the patch touches only the
//! `fixture`, `focus` and `imported from` entries whose content changed and
//! leaves every other byte alone.
//!
//! An entry is the span of lines from its keyword to the line before the
//! next entry, comment, blank line or the closing brace (the grammar lets an
//! entry wrap). A changed entry is replaced by its regenerated line, keeping
//! the trailing comment of its last line; an entry the venue no longer has is
//! removed; a new one is inserted before the closing brace. The result is
//! parsed back and must equal the venue asked for, or nothing is returned.

use super::parser::parse_venues;
use super::types::{fmt_vec3, Venue};

/// One entry of a venue block.
struct Entry {
    kind: Kind,
    /// The quoted name after `fixture` / `focus`; empty for the source line.
    name: String,
    start: usize,
    /// One past the last line.
    end: usize,
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Fixture,
    Focus,
    Source,
}

/// The text before a `#` that is outside quotes, and the comment from the
/// `#` on.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return (&line[..i], Some(&line[i..])),
            _ => {}
        }
    }
    (line, None)
}

fn entry_kind(code: &str) -> Option<Kind> {
    let t = code.trim_start();
    let word = t.split(|c: char| c.is_whitespace() || c == '"').next()?;
    match word {
        "fixture" => Some(Kind::Fixture),
        "focus" => Some(Kind::Focus),
        "imported" => Some(Kind::Source),
        _ => None,
    }
}

fn quoted_name(code: &str) -> String {
    let mut parts = code.splitn(3, '"');
    parts.next();
    parts.next().unwrap_or_default().to_string()
}

/// An entry's text as the writer would state it, so a hand-written entry
/// with the same content (fields in another order, wrapped) reads as
/// unchanged. Falls back to the text itself when it does not parse alone.
fn canonical(code: &str) -> String {
    let mini = format!("venue \"_\" {{\n{code}\n}}\n");
    let Some(venue) = parse_venues(&mini).ok().and_then(|mut v| v.remove("_")) else {
        return code.to_string();
    };
    if let Some(f) = venue.fixtures().values().next() {
        return format!("  {f}");
    }
    match venue.focus_points().iter().next() {
        Some((n, p)) => format!("  focus \"{n}\" {}", fmt_vec3(p)),
        None => code.to_string(),
    }
}

/// Text the caller wants written next to what the patch adds: the MVR
/// importer's `# layer` notes on new fixtures and its `# TODO` lines.
#[derive(Default)]
pub struct PatchNotes {
    /// Trailing comment (with its `#`) for a fixture the patch adds.
    pub fixture_comments: std::collections::HashMap<String, String>,
    /// Whole lines, written before the closing brace after everything new.
    pub trailing_lines: Vec<String>,
}

/// Patches `content` so the venue `name` in it becomes `desired`. Errors say
/// why the file could not be patched safely; the caller refuses the save.
pub fn patch_venue(content: &str, name: &str, desired: &Venue) -> Result<String, String> {
    patch_venue_with(content, name, desired, &PatchNotes::default())
}

/// [`patch_venue`] that also writes `notes` alongside what it adds.
pub fn patch_venue_with(
    content: &str,
    name: &str,
    desired: &Venue,
    notes: &PatchNotes,
) -> Result<String, String> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let header = format!("venue \"{name}\"");
    let open = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&header) && l.contains('{'))
        .ok_or_else(|| format!("cannot find the block of venue \"{name}\" to patch"))?;
    let close = lines[open + 1..]
        .iter()
        .position(|l| l.trim() == "}")
        .map(|i| i + open + 1)
        .ok_or_else(|| format!("cannot find the end of venue \"{name}\""))?;
    if lines[open].trim_end().ends_with('}') {
        return Err("a one-line venue block cannot be patched".to_string());
    }

    // --- Entries.
    let mut entries: Vec<Entry> = Vec::new();
    for (i, line) in lines.iter().enumerate().take(close).skip(open + 1) {
        let (code, _) = split_comment(line);
        if code.trim().is_empty() {
            continue;
        }
        match entry_kind(code) {
            Some(kind) => entries.push(Entry {
                kind,
                name: if kind == Kind::Source {
                    String::new()
                } else {
                    quoted_name(code)
                },
                start: i,
                end: i + 1,
            }),
            // A continuation of the entry above (wrapped onto more lines).
            None => match entries.last_mut() {
                Some(last) if last.end == i => last.end = i + 1,
                _ => {
                    return Err(format!(
                        "line {} of the venue is not one the patch understands",
                        i + 1
                    ))
                }
            },
        }
    }

    // --- Regenerated text of what the venue should hold.
    let want_fixture = |n: &str| desired.fixtures().get(n).map(|f| format!("  {f}"));
    let want_focus = |n: &str| {
        desired
            .focus_points()
            .get(n)
            .map(|p| format!("  focus \"{n}\" {}", fmt_vec3(p)))
    };
    let want_source = desired.source().map(|s| {
        format!(
            "  imported from mvr(\"{}\") origin {}",
            s.mvr,
            fmt_vec3(&s.origin)
        )
    });
    let normal = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut out = String::with_capacity(content.len() + 256);
    let mut seen_fixtures = std::collections::HashSet::new();
    let mut seen_focus = std::collections::HashSet::new();
    let mut had_source = false;
    let mut at = 0;
    for entry in &entries {
        out.extend(lines[at..entry.start].iter().copied());
        at = entry.end;
        let span = &lines[entry.start..entry.end];
        let regenerated = match entry.kind {
            Kind::Fixture => want_fixture(&entry.name),
            Kind::Focus => want_focus(&entry.name),
            Kind::Source => {
                had_source = true;
                want_source.clone()
            }
        };
        match entry.kind {
            Kind::Fixture => {
                seen_fixtures.insert(entry.name.clone());
            }
            Kind::Focus => {
                seen_focus.insert(entry.name.clone());
            }
            Kind::Source => {}
        }
        let Some(regenerated) = regenerated else {
            continue; // removed
        };
        let last = span[span.len() - 1];
        let (_, comment) = split_comment(last.trim_end_matches(['\n', '\r']));
        let code: String = span
            .iter()
            .map(|l| split_comment(l.trim_end_matches(['\n', '\r'])).0)
            .collect::<Vec<_>>()
            .join(" ");
        if normal(&canonical(&code)) == normal(&regenerated) {
            out.extend(span.iter().copied()); // unchanged: keep its bytes
        } else {
            out.push_str(&regenerated);
            if let Some(comment) = comment {
                out.push_str("  ");
                out.push_str(comment.trim_end());
            }
            out.push('\n');
        }
    }
    out.extend(lines[at..close].iter().copied());

    // --- What is new goes before the closing brace (a source line, first).
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    if !had_source {
        if let Some(source) = &want_source {
            // Right after the opening line.
            let head: usize = lines[..=open].iter().map(|l| l.len()).sum();
            let mut with = String::new();
            with.push_str(&out[..head]);
            with.push_str(source);
            with.push('\n');
            with.push_str(&out[head..]);
            out = with;
        }
    }
    for fixture in desired.fixtures_by_patch() {
        if !seen_fixtures.contains(fixture.name()) {
            out.push_str(&format!("  {fixture}"));
            if let Some(comment) = notes.fixture_comments.get(fixture.name()) {
                out.push_str("  ");
                out.push_str(comment);
            }
            out.push('\n');
        }
    }
    for (focus, point) in desired.focus_points() {
        if !seen_focus.contains(focus) {
            out.push_str(&format!("  focus \"{focus}\" {}\n", fmt_vec3(point)));
        }
    }
    for line in &notes.trailing_lines {
        out.push_str(line);
        out.push('\n');
    }
    out.extend(lines[close..].iter().copied());

    // --- The patch must read back as exactly the venue asked for.
    let reparsed =
        parse_venues(&out).map_err(|e| format!("the patched venue would not parse: {e}"))?;
    let after = reparsed
        .get(name)
        .ok_or_else(|| "the patched file lost the venue".to_string())?;
    if format!("{after}") != format!("{desired}") {
        return Err(
            "the patched venue did not read back as the one asked for; nothing written".to_string(),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::types::Fixture;

    const FILE: &str = r##"# House rig. Bricks hang on the front truss.
venue "house" {
  imported from mvr("lighting/library/house.mvr") origin (0, -3.5, 0)
  fixture "A" brick @ 1:1 tags ["wash"] position (1, 2, 3)  # layer "Front"
  fixture "B" brick @ 1:5
      position (2, 2, 3) rotation (0, 0, 90)
      tags ["x"]  # layer "Back"
  # TODO fixture "Lost" @ 2:1 position (0, 0, 0): no GDTF mode  # layer "Back"

  focus "mid" (0, 1, 0)
}
"##;

    fn venue(text: &str) -> Venue {
        parse_venues(text).unwrap().remove("house").unwrap()
    }

    fn with_fixture_tags(v: &Venue, name: &str, tags: &[&str]) -> Venue {
        let mut fixtures = v.fixtures().clone();
        let f = fixtures.get(name).unwrap().clone();
        let f = Fixture::new(
            f.name().to_string(),
            f.fixture_type().to_string(),
            f.universe(),
            f.start_channel(),
            tags.iter().map(|t| t.to_string()).collect(),
        )
        .with_position(f.position())
        .with_rotation(f.rotation());
        fixtures.insert(name.to_string(), f);
        Venue::new(v.name().to_string(), fixtures)
            .with_focus_points(v.focus_points().clone())
            .with_source(v.source().cloned())
    }

    #[test]
    fn a_tag_change_keeps_todo_header_and_trailing_comments() {
        let v = with_fixture_tags(&venue(FILE), "A", &["wash", "front"]);
        let out = patch_venue(FILE, "house", &v).unwrap();
        assert!(out.starts_with("# House rig. Bricks hang on the front truss.\n"));
        assert!(out.contains("# TODO fixture \"Lost\" @ 2:1"), "{out}");
        assert!(out.contains("tags [\"wash\", \"front\"]"), "{out}");
        assert!(out.contains("  # layer \"Front\"\n"), "{out}");
        assert!(
            out.contains("imported from mvr(\"lighting/library/house.mvr\") origin (0, -3.5, 0)\n")
        );
        // Untouched entries keep their bytes, wrapped one included.
        assert!(
            out.contains("  fixture \"B\" brick @ 1:5\n      position (2, 2, 3)"),
            "{out}"
        );
        assert!(out.contains("\n\n  focus \"mid\""), "{out}");
    }

    #[test]
    fn a_wrapped_entry_is_replaced_whole_and_keeps_its_comment() {
        let v = with_fixture_tags(&venue(FILE), "B", &["y"]);
        let out = patch_venue(FILE, "house", &v).unwrap();
        assert!(!out.contains("      position (2, 2, 3)"), "{out}");
        assert!(
            out.contains("tags [\"y\"] position (2, 2, 3) rotation (0, 0, 90)  # layer \"Back\"\n"),
            "{out}"
        );
        assert!(out.contains("# TODO fixture \"Lost\""));
    }

    #[test]
    fn a_removed_fixture_goes_and_nothing_else_moves() {
        let v = venue(FILE);
        let mut fixtures = v.fixtures().clone();
        fixtures.remove("A");
        let v = Venue::new("house".into(), fixtures)
            .with_focus_points(v.focus_points().clone())
            .with_source(v.source().cloned());
        let out = patch_venue(FILE, "house", &v).unwrap();
        let expected = FILE.replace(
            "  fixture \"A\" brick @ 1:1 tags [\"wash\"] position (1, 2, 3)  # layer \"Front\"\n",
            "",
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn a_new_focus_point_and_fixture_are_added_before_the_brace() {
        let v = venue(FILE);
        let mut focus = v.focus_points().clone();
        focus.insert("drums".into(), [0.0, 4.0, 1.0]);
        let mut fixtures = v.fixtures().clone();
        fixtures.insert(
            "C".into(),
            Fixture::new("C".into(), "brick".into(), 1, 9, vec![]),
        );
        let v = Venue::new("house".into(), fixtures)
            .with_focus_points(focus)
            .with_source(v.source().cloned());
        let out = patch_venue(FILE, "house", &v).unwrap();
        assert!(
            out.contains("  fixture \"C\" brick @ 1:9\n  focus \"drums\" (0, 4, 1)\n}\n"),
            "{out}"
        );
        assert!(out.starts_with("# House rig"));
    }

    #[test]
    fn a_patch_that_would_not_round_trip_is_refused() {
        // A stray line the patch cannot place.
        let bad = FILE.replace("  focus \"mid\"", "  garbage here\n  focus \"mid\"");
        let v = venue(FILE);
        assert!(patch_venue(&bad, "house", &v).is_err());
        assert!(patch_venue(FILE, "other", &v).is_err());
    }
}
