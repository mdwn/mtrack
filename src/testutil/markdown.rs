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
//! Reading the docs' markdown tables, for the tests that hold a table to
//! the code it restates.

/// A markdown table: its header cells and each row's cells, as written
/// (trimmed; an escaped `\|` read as `|`).
#[derive(Debug, Clone)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// The column headed `name`.
    pub fn column(&self, name: &str) -> usize {
        self.header
            .iter()
            .position(|h| h == name)
            .unwrap_or_else(|| panic!("no column {name} in {:?}", self.header))
    }

    /// Each row as (first cell without its backticks, cell of `column`
    /// without its backticks).
    pub fn pairs(&self, column: &str) -> Vec<(String, String)> {
        let c = self.column(column);
        self.rows
            .iter()
            .map(|row| (plain(&row[0]), plain(&row[c])))
            .collect()
    }
}

/// The cells of one table line.
fn cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => out.push(std::mem::take(&mut cell).trim().to_string()),
            _ => cell.push(c),
        }
    }
    out.push(cell.trim().to_string());
    out
}

/// Every table in `markdown`, in order.
pub fn tables(markdown: &str) -> Vec<Table> {
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        let is_rule = |l: &str| {
            let l = l.trim();
            l.starts_with('|') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
        };
        if lines[i].trim().starts_with('|') && is_rule(lines[i + 1]) {
            let header = cells(lines[i]);
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(cells(lines[i]));
                i += 1;
            }
            out.push(Table { header, rows });
        } else {
            i += 1;
        }
    }
    out
}

/// The first table in `markdown` whose header is `header`.
pub fn table(markdown: &str, header: &[&str]) -> Table {
    tables(markdown)
        .into_iter()
        .find(|t| t.header == header)
        .unwrap_or_else(|| panic!("no table headed {header:?}"))
}

/// The tables in `markdown` whose header is `header`, in order.
pub fn tables_headed(markdown: &str, header: &[&str]) -> Vec<Table> {
    tables(markdown)
        .into_iter()
        .filter(|t| t.header == header)
        .collect()
}

/// A cell's text without its code marks and its emphasis (a `*` inside,
/// as in an address pattern, is kept).
pub fn plain(cell: &str) -> String {
    cell.replace('`', "")
        .trim()
        .trim_matches('*')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_table_with_escaped_pipes() {
        let md = "text\n\n| Key | Value |\n|---|:--:|\n| `a` | `int\\|float` |\n| b | 2 |\n\nafter";
        let t = &tables(md)[0];
        assert_eq!(t.header, ["Key", "Value"]);
        assert_eq!(t.rows[0], ["`a`", "`int|float`"]);
        assert_eq!(
            t.pairs("Value"),
            [("a".into(), "int|float".into()), ("b".into(), "2".into())]
        );
    }
}
