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

/// A pest error on `content` as the parser reports it: where, what, and the
/// lines around it.
pub(crate) fn describe_pest_error(
    e: &pest::error::Error<super::grammar::Rule>,
    content: &str,
) -> String {
    describe_pest_error_in("DSL", e, content)
}

/// [`describe_pest_error`] for a file of `what` ("Fixture types DSL"),
/// which names it.
pub(crate) fn describe_pest_error_in(
    what: &str,
    e: &pest::error::Error<super::grammar::Rule>,
    content: &str,
) -> String {
    let (line, col) = match e.line_col {
        pest::error::LineColLocation::Pos(pos) => pos,
        pest::error::LineColLocation::Span(pos, _) => pos,
    };
    // An unquoted colour reads as a comment, so the parse fails somewhere
    // after it (often the next line) on a value that is simply missing.
    // Pointed at where the colour is, with why.
    if let Some((line, col, hex)) = unquoted_hex_colour(content, line) {
        return format!(
            "{what} parsing error at line {line}, column {col}: the colour {hex} must be in \
             quotes, \"{hex}\": outside quotes, # starts a comment, so the rest of the line \
             is not read\n\nContent around error:\n{}",
            get_error_context(content, line, col)
        );
    }
    format!(
        "{what} parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
        line,
        col,
        e.variant.message(),
        get_error_context(content, line, col)
    )
}

/// The last hex colour written without quotes (`primary: #ff8800`) on or
/// before line `upto` (1-based): where its `#` is (line, column, 1-based)
/// and the colour. Only a `#` outside quotes, right after `:`, `[`, `,` or
/// `(`, and followed by 3, 6 or 8 hex digits counts, so a comment (`# note`,
/// `#fade` after a cue) is not taken for one.
fn unquoted_hex_colour(content: &str, upto: usize) -> Option<(usize, usize, String)> {
    let mut found = None;
    for (i, text) in content.lines().take(upto).enumerate() {
        let mut in_quotes = false;
        let mut previous = None;
        for (at, c) in text.char_indices() {
            match c {
                '"' => in_quotes = !in_quotes,
                '#' if !in_quotes => {
                    let digits: String = text[at + 1..]
                        .chars()
                        .take_while(char::is_ascii_alphanumeric)
                        .collect();
                    let is_hex = matches!(digits.len(), 3 | 6 | 8)
                        && digits.chars().all(|d| d.is_ascii_hexdigit());
                    if is_hex && matches!(previous, Some(':' | '[' | ',' | '(')) {
                        let col = text[..at].chars().count() + 1;
                        found = Some((i + 1, col, format!("#{digits}")));
                    }
                    // The rest of the line is a comment either way.
                    break;
                }
                _ => {}
            }
            if !c.is_whitespace() {
                previous = Some(c);
            }
        }
    }
    found
}

pub(crate) fn get_error_context(content: &str, line: usize, col: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();

    if line == 0 || line > lines.len() {
        return "Unable to determine error context".to_string();
    }

    let error_line = line - 1; // Convert to 0-based index
    let start_line = error_line.saturating_sub(2);
    let end_line = if error_line + 2 < lines.len() {
        error_line + 2
    } else {
        lines.len() - 1
    };

    let mut context = String::new();

    for (i, line_content) in lines.iter().enumerate().take(end_line + 1).skip(start_line) {
        let line_num = i + 1;

        if i == error_line {
            // Highlight the error line
            context.push_str(&format!("{:4} | {}\n", line_num, line_content));
            context.push_str(&format!("     | {}^", " ".repeat(col.saturating_sub(1))));
            // The lines after it each start on their own.
            if i < end_line {
                context.push('\n');
            }
        } else {
            context.push_str(&format!("{:4} | {}\n", line_num, line_content));
        }
    }

    context
}

/// Analyze why parsing failed and provide helpful suggestions
pub(crate) fn analyze_parsing_failure(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut suggestions = Vec::new();

    // Check for common issues
    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let trimmed = line.trim();

        // Check for missing quotes around show names
        if trimmed.starts_with("show") && !trimmed.contains('"') {
            suggestions.push(format!(
                "Line {}: Show name appears to be missing quotes around the name",
                line_num
            ));
        }

        // Check for missing @ symbol before time
        if trimmed.starts_with("00:") || trimmed.starts_with("0:") {
            suggestions.push(format!(
                "Line {}: Time appears to be missing @ symbol (e.g., @00:00.000)",
                line_num
            ));
        }

        // Check for common typos — "shows" without the singular keyword "show " or "show\""
        if trimmed.contains("shows") && !trimmed.contains("show ") && !trimmed.contains("show\"") {
            suggestions.push(format!(
                "Line {}: Did you mean 'show' instead of 'shows'?",
                line_num
            ));
        }
    }

    if suggestions.is_empty() {
        "Unable to determine specific parsing issues. Please check the syntax against the DSL documentation.".to_string()
    } else {
        format!("Possible issues found:\n{}", suggestions.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── unquoted hex colours ───────────────────────────────────────

    /// The palette slot written as `primary: #ff8800` is reported on its own
    /// line, with the fix, not on the line after it where the parse stopped.
    #[test]
    fn an_unquoted_palette_colour_is_reported_where_it_is() {
        let content = "palette {\n    primary: #ff8800\n    accent: \"blue\"\n}\n\n\
                       show \"S\" {\n    @00:00.000\n    \
                       wash: static color: palette.primary, duration: 1s\n}\n";
        let err = crate::lighting::parser::parse_light_shows(content).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("line 2, column 14"), "{err}");
        assert!(
            err.contains("the colour #ff8800 must be in quotes, \"#ff8800\""),
            "{err}"
        );
    }

    /// An effect's colour too, which the parse otherwise trips over at the
    /// cue after it.
    #[test]
    fn an_unquoted_effect_colour_is_reported_where_it_is() {
        let content = "show \"S\" {\n    @00:00.000\n    \
                       wash: static color: #f00, duration: 1s\n\
                       @00:01.000\n    wash: static color: \"blue\", duration: 1s\n}\n";
        let err = crate::lighting::parser::parse_light_shows(content).unwrap_err();
        let err = err.to_string();
        assert!(err.contains("line 3, column 25"), "{err}");
        assert!(err.contains("the colour #f00 must be in quotes"), "{err}");
    }

    /// A comment, or a `#` inside quotes, is not taken for a colour: an
    /// error elsewhere is reported as it was.
    #[test]
    fn comments_and_quoted_colours_are_not_taken_for_one() {
        let content = "# fade the wash in\nshow \"S\" { #add more cues\n    @00:00.000\n    \
                       wash: static color: \"#ff0000\", duration: 1s,\n}\n";
        let err = crate::lighting::parser::parse_light_shows(content).unwrap_err();
        let err = err.to_string();
        assert!(!err.contains("must be in quotes"), "{err}");
    }

    // ── get_error_context ──────────────────────────────────────────

    /// The line after the marked one starts on its own line, not after the
    /// caret.
    #[test]
    fn error_context_lines_after_the_caret_start_on_their_own() {
        let ctx = get_error_context("one\ntwo\nthree", 2, 2);
        assert_eq!(ctx, "   1 | one\n   2 | two\n     |  ^\n   3 | three\n");
    }

    #[test]
    fn error_context_basic() {
        let content = "line1\nline2\nline3\nline4\nline5";
        let ctx = get_error_context(content, 3, 1);
        assert!(ctx.contains("line3"));
        assert!(ctx.contains("^")); // caret marker
    }

    #[test]
    fn error_context_first_line() {
        let content = "first\nsecond\nthird";
        let ctx = get_error_context(content, 1, 3);
        assert!(ctx.contains("first"));
        assert!(ctx.contains("^"));
    }

    #[test]
    fn error_context_last_line() {
        let content = "first\nsecond\nthird";
        let ctx = get_error_context(content, 3, 1);
        assert!(ctx.contains("third"));
        assert!(ctx.contains("^"));
    }

    #[test]
    fn error_context_line_zero_invalid() {
        let ctx = get_error_context("hello", 0, 1);
        assert!(ctx.contains("Unable to determine"));
    }

    #[test]
    fn error_context_line_out_of_bounds() {
        let ctx = get_error_context("hello", 100, 1);
        assert!(ctx.contains("Unable to determine"));
    }

    #[test]
    fn error_context_col_offset() {
        let content = "line1\nline2\nline3";
        let ctx = get_error_context(content, 2, 5);
        // Should have spaces before the caret
        assert!(ctx.contains("    ^"));
    }

    #[test]
    fn error_context_col_one() {
        let content = "hello";
        let ctx = get_error_context(content, 1, 1);
        assert!(ctx.contains("^"));
    }

    // ── analyze_parsing_failure ────────────────────────────────────

    #[test]
    fn analyze_no_issues() {
        let result = analyze_parsing_failure("normal content here");
        assert!(result.contains("Unable to determine"));
    }

    #[test]
    fn analyze_missing_show_quotes() {
        let result = analyze_parsing_failure("show My Show Name");
        assert!(result.contains("missing quotes"));
    }

    #[test]
    fn analyze_show_with_quotes_ok() {
        // show "My Show" should NOT trigger the warning
        let result = analyze_parsing_failure("show \"My Show\"");
        assert!(result.contains("Unable to determine"));
    }

    #[test]
    fn analyze_missing_at_symbol() {
        let result = analyze_parsing_failure("00:30.000");
        assert!(result.contains("missing @"));
    }

    #[test]
    fn analyze_missing_at_symbol_short() {
        let result = analyze_parsing_failure("0:15");
        assert!(result.contains("missing @"));
    }

    #[test]
    fn analyze_shows_typo() {
        let result = analyze_parsing_failure("shows something");
        assert!(result.contains("Did you mean 'show'"));
    }

    #[test]
    fn analyze_multiple_issues() {
        let content = "show My Show\n00:30.000";
        let result = analyze_parsing_failure(content);
        assert!(result.contains("Possible issues found:"));
        // Should find both issues
        assert!(result.contains("missing quotes"));
        assert!(result.contains("missing @"));
    }
}
