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

/// Get context around an error location for better error reporting
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

    // ── get_error_context ──────────────────────────────────────────

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
