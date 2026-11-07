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

//! A GDTF fixture's record, patched in place (venue-exchange design §22,
//! lighting UI design §12.4): the two things in it that are the user's —
//! the type's name, its movement limits and its strobe curve — are changed
//! where they are
//! written, and nothing else moves. The leading
//! comments, the archive path as written, other types in the file and any
//! body the settings do not own survive byte for byte. The result is parsed
//! back before it is handed over, so a patch that would not load is an
//! error, never a file.

use pest::Parser;

use super::parser::grammar::{LightingParser, Rule};
use super::parser::parse_fixture_types;
use super::parser::utils::items;
use super::types::{MovementLimits, StrobeCurve};

/// Whether the web UI's channel-map form can say everything `text`, a
/// fixture type file, says about the type `name`, so that a form save loses
/// nothing:
/// - the file holds that one type and nothing else, and no comment (by the
///   grammar's comment rule, [`super::parser::document::has_comment`]);
/// - the type is its channel map and the three strobe numbers alone: no
///   rich channel, GDTF source, movement limits, strobe curve, special
///   cases or anything else the form does not carry;
/// - each of those at most once, no channel twice in the map, and a
///   `channels` count, if any, that is the map's.
///
/// The one rule for which editor a type opens in, and for whether a form
/// save of it is taken.
pub fn form_expresses(text: &str, name: &str) -> bool {
    use super::files::{Contents, Editable, Kind};
    let alone = Contents::parse(Kind::FixtureType, text)
        .ok()
        .and_then(|contents| contents.alone(name))
        == Some(Editable::Editable);
    if !alone {
        return false;
    }
    let Ok(pairs) = LightingParser::parse(Rule::file, text) else {
        return false;
    };
    if super::parser::document::has_comment(text, pairs.clone()) {
        return false;
    }
    let Some(declaration) = items(pairs)
        .filter(|p| p.as_rule() == Rule::fixture_type)
        .find(|p| {
            p.clone()
                .into_inner()
                .find(|c| c.as_rule() == Rule::fixture_type_name)
                .is_some_and(|n| n.as_str().trim_matches('"') == name)
        })
    else {
        return false;
    };
    let mut fields = Vec::new();
    for child in declaration.into_inner() {
        match child.as_rule() {
            Rule::fixture_type_name => {}
            Rule::fixture_type_content => fields.extend(child.into_inner()),
            // A GDTF source, or anything else the form does not write.
            _ => return false,
        }
    }
    // What the form writes, each at most once.
    let mut seen = std::collections::HashSet::new();
    let mut count: Option<f64> = None;
    let mut keys: Vec<String> = Vec::new();
    for field in fields {
        let rule = field.as_rule();
        let known = matches!(
            rule,
            Rule::channels
                | Rule::channel_map
                | Rule::max_strobe_frequency
                | Rule::min_strobe_frequency
                | Rule::strobe_dmx_offset
        );
        if !known || !seen.insert(rule) {
            return false;
        }
        match rule {
            Rule::channels => {
                count = field
                    .into_inner()
                    .next()
                    .and_then(|n| n.as_str().trim().parse().ok());
                if count.is_none() {
                    return false;
                }
            }
            Rule::channel_map => {
                for mapping in field.into_inner().flat_map(|list| list.into_inner()) {
                    if let Some(key) = mapping
                        .into_inner()
                        .find(|p| p.as_rule() == Rule::channel_name)
                    {
                        keys.push(key.as_str().trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    let distinct: std::collections::HashSet<&String> = keys.iter().collect();
    distinct.len() == keys.len() && count.is_none_or(|n| n == keys.len() as f64)
}

/// What the fixture page's settings form owns in a type's file.
#[derive(Clone, Debug, PartialEq)]
pub struct FixtureSettings {
    /// The type's name: what venue fixture lines and shows call it.
    pub name: String,
    /// Movement limits; both `None` removes the block.
    pub movement: MovementLimits,
    /// The strobe curve; `None` (automatic) removes the statement.
    pub strobe_curve: Option<StrobeCurve>,
}

/// Patches the declaration of `current` in `content` to `settings`.
pub fn patch_fixture_type(
    content: &str,
    current: &str,
    settings: &FixtureSettings,
) -> Result<String, String> {
    let plain = |what: &str, text: &str| {
        if text.is_empty() || text.contains('"') || text.contains('\n') {
            Err(format!(
                "the {what} must be non-empty and hold no quote or line break"
            ))
        } else {
            Ok(())
        }
    };
    plain("name", &settings.name)?;
    for (what, value) in [
        ("max pan speed", settings.movement.max_pan_speed),
        ("max tilt speed", settings.movement.max_tilt_speed),
    ] {
        if value.is_some_and(|v| !v.is_finite() || v <= 0.0) {
            return Err(format!("the {what} must be a positive number of deg/s"));
        }
    }

    let pairs = LightingParser::parse(Rule::file, content)
        .map_err(|e| format!("the file does not parse: {e}"))?;
    let declaration = items(pairs)
        .filter(|p| p.as_rule() == Rule::fixture_type)
        .find(|p| {
            p.clone()
                .into_inner()
                .find(|c| c.as_rule() == Rule::fixture_type_name)
                .is_some_and(|n| n.as_str().trim_matches('"') == current)
        })
        .ok_or_else(|| format!("the file declares no fixture type \"{current}\""))?;

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let span = declaration.as_span();
    let mut name = None;
    let mut source = None;
    let mut body = None;
    for child in declaration.clone().into_inner() {
        match child.as_rule() {
            Rule::fixture_type_name => name = Some(child),
            Rule::gdtf_source => source = Some(child),
            Rule::fixture_type_content => body = Some(child),
            _ => {}
        }
    }
    let name = name.ok_or("the declaration has no name")?;
    source.ok_or_else(|| {
        format!("fixture type \"{current}\" is not GDTF-sourced; edit it as text")
    })?;

    // The name, in its quotes.
    edits.push((
        name.as_span().start(),
        name.as_span().end(),
        format!("\"{}\"", settings.name),
    ));

    // Movement limits and the strobe curve: each statement replaced,
    // removed (with its line, when it had the line to itself) or added on a
    // line of its own before the closing brace.
    let find = |rule: Rule| {
        body.clone()
            .and_then(|b| b.into_inner().find(|p| p.as_rule() == rule))
    };
    let curve = settings
        .strobe_curve
        .map(|c| format!("strobe_curve: {}", c.keyword()));
    for (statement, wanted) in [
        (
            find(Rule::movement_block),
            movement_text(&settings.movement),
        ),
        (find(Rule::strobe_curve), curve),
    ] {
        statement_edit(content, span.end(), statement, wanted, &mut edits)?;
    }

    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = content.to_string();
    for (start, end, text) in edits {
        out.replace_range(start..end, &text);
    }

    // Read it back: the patch must load as what was asked for.
    let types =
        parse_fixture_types(&out).map_err(|e| format!("the patched file does not parse: {e}"))?;
    let patched = types
        .get(&settings.name)
        .ok_or("the patched file lost the declaration")?;
    let ok = patched.source().is_some()
        && *patched.movement() == settings.movement
        && patched.strobe_curve() == settings.strobe_curve;
    if !ok {
        return Err("the patched file does not read back as the settings".to_string());
    }
    Ok(out)
}

/// The edit that makes one body statement `wanted`: replaced, removed
/// (with its line, when it had the line to itself) or added on a line of
/// its own before the declaration's closing brace (`end` is just past it).
fn statement_edit(
    content: &str,
    end_of_declaration: usize,
    statement: Option<pest::iterators::Pair<'_, Rule>>,
    wanted: Option<String>,
    edits: &mut Vec<(usize, usize, String)>,
) -> Result<(), String> {
    match (statement, wanted) {
        (Some(block), Some(text)) => {
            edits.push((block.as_span().start(), block.as_span().end(), text));
        }
        (Some(block), None) => {
            let (start, end) = (block.as_span().start(), block.as_span().end());
            let line_start = content[..start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = content[end..]
                .find('\n')
                .map_or(content.len(), |i| end + i + 1);
            let alone = content[line_start..start].trim().is_empty()
                && content[end..line_end].trim().is_empty();
            if alone {
                edits.push((line_start, line_end, String::new()));
            } else {
                edits.push((start, end, String::new()));
            }
        }
        (None, Some(text)) => {
            let brace = end_of_declaration - 1;
            if content.as_bytes().get(brace) != Some(&b'}') {
                return Err("cannot find the end of the declaration".to_string());
            }
            let line_start = content[..brace].rfind('\n').map_or(0, |i| i + 1);
            if content[line_start..brace].trim().is_empty() {
                edits.push((line_start, line_start, format!("  {text}\n")));
            } else {
                edits.push((brace, brace, format!("\n  {text}\n")));
            }
        }
        (None, None) => {}
    }
    Ok(())
}

/// The settings a file holds for a type: what the form starts from.
pub fn current_settings(content: &str, name: &str) -> Option<FixtureSettings> {
    let types = parse_fixture_types(content).ok()?;
    let fixture_type = types.get(name)?;
    fixture_type.source()?;
    Some(FixtureSettings {
        name: name.to_string(),
        movement: *fixture_type.movement(),
        strobe_curve: fixture_type.strobe_curve(),
    })
}

/// `movement { ... }` for the limits set; `None` when none is.
fn movement_text(limits: &MovementLimits) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(v) = limits.max_pan_speed {
        parts.push(format!("max_pan_speed: {}deg/s", number(v)));
    }
    if let Some(v) = limits.max_tilt_speed {
        parts.push(format!("max_tilt_speed: {}deg/s", number(v)));
    }
    (!parts.is_empty()).then(|| format!("movement {{ {} }}", parts.join(" ")))
}

/// A speed as the grammar takes it: digits, a decimal part only when there
/// is one.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v < 1e15 {
        format!("{}", v as u64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Imported from pb15.gdtf (\"PB15\" by Astera).\n\
                        # Channels come from the GDTF; this file carries only overrides.\n\
                        fixture_type \"Astera-PixelBrick\"\n  \
                        from gdtf(\"lighting/library/pb15.gdtf\"  )\n{\n  \
                        # measured on the rig\n  \
                        special_cases: [\"Wash\"]\n}\n";

    fn settings(name: &str, pan: Option<f64>, tilt: Option<f64>) -> FixtureSettings {
        FixtureSettings {
            name: name.to_string(),
            movement: MovementLimits {
                max_pan_speed: pan,
                max_tilt_speed: tilt,
            },
            strobe_curve: None,
        }
    }

    fn curve(curve: Option<StrobeCurve>) -> FixtureSettings {
        FixtureSettings {
            strobe_curve: curve,
            ..settings("Astera-PixelBrick", None, None)
        }
    }

    #[test]
    fn the_strobe_curve_is_added_changed_and_removed_keeping_comments() {
        let added =
            patch_fixture_type(FILE, "Astera-PixelBrick", &curve(Some(StrobeCurve::Linear)))
                .unwrap();
        assert!(
            added.ends_with(
                "  # measured on the rig
  special_cases: [\"Wash\"]
  strobe_curve: linear
}
"
            ),
            "{added}"
        );
        assert_eq!(
            current_settings(&added, "Astera-PixelBrick")
                .unwrap()
                .strobe_curve,
            Some(StrobeCurve::Linear)
        );
        // Beside movement limits, each on its own line; changing one leaves
        // the other.
        let both = patch_fixture_type(
            &added,
            "Astera-PixelBrick",
            &FixtureSettings {
                strobe_curve: Some(StrobeCurve::Linear),
                ..settings("Astera-PixelBrick", Some(240.0), None)
            },
        )
        .unwrap();
        assert!(both.contains("  strobe_curve: linear\n"), "{both}");
        assert!(
            both.contains("  movement { max_pan_speed: 240deg/s }\n"),
            "{both}"
        );
        // A comment on the statement's line stays, and so does the line.
        let commented = both.replace(
            "strobe_curve: linear",
            "strobe_curve: linear # measured 2026-10",
        );
        let removed = patch_fixture_type(
            &commented,
            "Astera-PixelBrick",
            &settings("Astera-PixelBrick", Some(240.0), None),
        )
        .unwrap();
        assert!(!removed.contains("strobe_curve"), "{removed}");
        assert!(removed.contains("  # measured 2026-10\n"), "{removed}");
        // Changed in place.
        let changed = patch_fixture_type(
            &added,
            "Astera-PixelBrick",
            &curve(Some(StrobeCurve::Declared)),
        )
        .unwrap();
        assert_eq!(changed, added.replace("linear", "declared"));
        // Automatic: the statement goes.
        let back = patch_fixture_type(&added, "Astera-PixelBrick", &curve(None)).unwrap();
        assert_eq!(back, FILE);
    }

    #[test]
    fn the_current_settings_are_unchanged_by_a_patch_to_themselves() {
        let current = current_settings(FILE, "Astera-PixelBrick").unwrap();
        assert_eq!(current.name, "Astera-PixelBrick");
        assert_eq!(
            patch_fixture_type(FILE, "Astera-PixelBrick", &current).unwrap(),
            FILE
        );
    }

    #[test]
    fn a_rename_keeps_comments_path_and_body() {
        let out =
            patch_fixture_type(FILE, "Astera-PixelBrick", &settings("Brick", None, None)).unwrap();
        assert_eq!(
            out,
            FILE.replace(
                "fixture_type \"Astera-PixelBrick\"",
                "fixture_type \"Brick\""
            )
        );
    }

    #[test]
    fn movement_limits_are_added_replaced_and_removed_on_their_own_line() {
        let added = patch_fixture_type(
            FILE,
            "Astera-PixelBrick",
            &settings("Astera-PixelBrick", Some(240.0), None),
        )
        .unwrap();
        assert!(
            added.ends_with(
                "  special_cases: [\"Wash\"]\n  movement { max_pan_speed: 240deg/s }\n}\n"
            ),
            "{added}"
        );
        let replaced = patch_fixture_type(
            &added,
            "Astera-PixelBrick",
            &settings("Astera-PixelBrick", Some(240.0), Some(180.5)),
        )
        .unwrap();
        assert!(
            replaced
                .contains("  movement { max_pan_speed: 240deg/s max_tilt_speed: 180.5deg/s }\n"),
            "{replaced}"
        );
        let removed = patch_fixture_type(
            &replaced,
            "Astera-PixelBrick",
            &settings("Astera-PixelBrick", None, None),
        )
        .unwrap();
        assert_eq!(removed, FILE);
    }

    #[test]
    fn an_empty_body_gains_its_block_inside_the_braces() {
        let file = "fixture_type \"B\" from gdtf(\"a.gdtf\") {\n}\n";
        let out = patch_fixture_type(file, "B", &settings("B", None, Some(90.0))).unwrap();
        assert_eq!(
            out,
            "fixture_type \"B\" from gdtf(\"a.gdtf\") {\n  movement { max_tilt_speed: 90deg/s }\n}\n"
        );
        let one_line = "fixture_type \"B\" from gdtf(\"a.gdtf\") {}\n";
        let out = patch_fixture_type(one_line, "B", &settings("B", Some(1.0), None)).unwrap();
        assert!(
            current_settings(&out, "B").unwrap().movement.max_pan_speed == Some(1.0),
            "{out}"
        );
    }

    #[test]
    fn another_type_in_the_file_is_left_alone() {
        let file = format!("{FILE}\nfixture_type \"Other\" from gdtf(\"b.gdtf\") {{\n}}\n");
        let out = patch_fixture_type(&file, "Other", &settings("Renamed", None, None)).unwrap();
        assert!(out.starts_with(FILE), "{out}");
        assert!(
            out.ends_with("fixture_type \"Renamed\" from gdtf(\"b.gdtf\") {\n}\n"),
            "{out}"
        );
    }

    #[test]
    fn refusals_say_why() {
        let bad =
            |s: FixtureSettings| patch_fixture_type(FILE, "Astera-PixelBrick", &s).unwrap_err();
        assert!(bad(settings("a\"b", None, None)).contains("name"));
        assert!(bad(settings("B", Some(-1.0), None)).contains("pan"));
        assert!(patch_fixture_type(FILE, "Nope", &settings("B", None, None))
            .unwrap_err()
            .contains("Nope"));
        let native = "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n";
        assert!(
            patch_fixture_type(native, "Par", &settings("Par", None, None))
                .unwrap_err()
                .contains("not GDTF-sourced")
        );
    }

    #[test]
    fn the_form_expresses_no_repeat_and_only_the_maps_own_count() {
        let typed = |body: &str| format!("fixture_type \"Par\" {{\n{body}}}\n");
        let map = "  channel_map: { \"red\": 1, \"green\": 2 }\n";
        assert!(form_expresses(&typed(map), "Par"));
        assert!(form_expresses(
            &typed(&format!("  channels: 2\n{map}")),
            "Par"
        ));
        for (what, body) in [
            (
                "a count that is not the map's",
                format!("  channels: 3\n{map}"),
            ),
            (
                "a second map",
                format!("{map}  channel_map: {{ \"blue\": 3 }}\n"),
            ),
            (
                "a repeated key",
                "  channel_map: { \"red\": 1, \"red\": 2 }\n".to_string(),
            ),
            (
                "a repeated strobe field",
                format!("{map}  max_strobe_frequency: 20.0\n  max_strobe_frequency: 25.0\n"),
            ),
            (
                "a repeated count",
                format!("  channels: 2\n  channels: 2\n{map}"),
            ),
        ] {
            let text = typed(&body);
            assert!(!form_expresses(&text, "Par"), "{what}: {text}");
        }
    }

    #[test]
    fn the_form_expresses_only_a_plain_type_alone_in_its_file() {
        let plain = "fixture_type \"Par\" {\n  channels: 2\n  channel_map: { \"dimmer\": 1, \"strobe\": 2 }\n  max_strobe_frequency: 20.0\n  min_strobe_frequency: 0.5\n  strobe_dmx_offset: 7\n}\n";
        assert!(form_expresses(plain, "Par"));
        let with = |line: &str| {
            plain.replace(
                "  strobe_dmx_offset: 7\n",
                &format!("  strobe_dmx_offset: 7\n  {line}\n"),
            )
        };
        for (what, text) in [
            ("a comment line", format!("# mine\n{plain}")),
            (
                "a trailing comment",
                plain.replace("channels: 2", "channels: 2 // two"),
            ),
            ("a strobe curve", with("strobe_curve: linear")),
            ("special cases", with("special_cases: [\"Wash\"]")),
            (
                "movement limits",
                with("movement { max_pan_speed: 90deg/s }"),
            ),
            (
                "a rich channel",
                "fixture_type \"Par\" {\n  channel \"pan\" @ 1 fine 2\n}\n".to_string(),
            ),
            (
                "another type",
                format!("{plain}{}", plain.replace("\"Par\"", "\"Wash\"")),
            ),
            (
                "a GDTF source",
                "fixture_type \"Par\" from gdtf(\"lighting/library/x.gdtf\") {\n}\n".to_string(),
            ),
        ] {
            assert!(!form_expresses(&text, "Par"), "{what}: {text}");
        }
        // A `#` inside a name is not a comment.
        let hash = plain.replace("\"Par\"", "\"Par #3\"");
        assert!(form_expresses(&hash, "Par #3"));
        // A string over two lines does not hide a comment after it: the
        // grammar's comment rule finds it, not a line scan.
        let spanning =
            "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dim\nmer\": 1 } # mine\n}\n";
        assert!(crate::lighting::parser::parse_fixture_types(spanning).is_ok());
        {
            assert!(!form_expresses(spanning, "Par"), "{spanning}");
            assert!(form_expresses(&spanning.replace(" # mine", ""), "Par"));
        }
        let spanning_name =
            "fixture_type \"Par\n#1\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n";
        assert!(crate::lighting::parser::parse_fixture_types(spanning_name).is_ok());
        {
            assert!(form_expresses(spanning_name, "Par\n#1"));
            let commented = spanning_name.replace("channels: 1", "channels: 1 # mine");
            assert!(!form_expresses(&commented, "Par\n#1"));
        }
        assert!(!form_expresses(plain, "Other"));
    }
}
