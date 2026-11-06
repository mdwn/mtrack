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
//! The docs' tables and lists that restate the code, held to it: each test
//! reads a table out of `docs/src` and compares it with the constants and
//! defaults it restates, so a change to either side alone fails.

mod config;
mod effects;
mod positions;

/// A value as a table states it, for comparing with the code's: a number
/// (with its unit, `%` or `s`), or a word.
#[derive(Debug, PartialEq)]
enum Stated {
    Number(f64, &'static str),
    Word(String),
}

/// `text` (backticks, emphasis and trailing punctuation dropped) read as a
/// [`Stated`].
fn stated(text: &str) -> Stated {
    let text = text
        .replace(['`', '*', '"'], "")
        .trim()
        .trim_end_matches(['.', ',', ';', ')'])
        .to_string();
    for unit in ["%", "ms", "s", ""] {
        if let Some(n) = text.strip_suffix(unit) {
            if let Ok(n) = n.trim().parse::<f64>() {
                return Stated::Number(n, unit);
            }
        }
    }
    Stated::Word(text)
}

#[test]
fn stated_values_compare_as_numbers() {
    assert_eq!(stated("`1.0`"), stated("1"));
    assert_eq!(stated("50%"), Stated::Number(50.0, "%"));
    assert_eq!(stated("**1s**"), Stated::Number(1.0, "s"));
    assert_eq!(stated("`linear`"), Stated::Word("linear".into()));
}
