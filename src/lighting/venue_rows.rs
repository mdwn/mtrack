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
////! What stops a venue editor's fixture rows, or a fixture type editor's
//! channel rows, from saving: the one check the save makes and the editor
//! marks each row by. Light, so the web UI's WebAssembly module compiles it
//! as it is.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A venue's fixture row, as the editor holds it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct VenueRow {
    pub name: String,
    pub fixture_type: String,
    pub universe: f64,
    pub start_channel: f64,
    #[serde(default)]
    #[cfg_attr(feature = "tsify", tsify(optional, type = "string | null"))]
    pub mode: Option<String>,
}

/// Why a fixture row cannot be saved (the editor's
/// `lighting.venueRow.<problem>`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RowProblem {
    NoName,
    /// Another row has the same name, spaces either side aside.
    DuplicateName,
    NoType,
    /// A fixture from a GDTF names no mode.
    NoMode,
    /// Not a whole number from 1 to 65535.
    BadUniverse,
    BadAddress,
}

/// A row and what stops it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RowProblems {
    pub row: u32,
    pub problems: Vec<RowProblem>,
}

/// A fixture type's channel row, as the editor holds it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ChannelRow {
    pub name: String,
    pub offset: f64,
}

/// Why a channel row cannot be saved (the editor's
/// `lighting.channelRow.<problem>`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChannelProblem {
    NoName,
    DuplicateName,
    /// Not a whole number from 1 to 65535.
    BadOffset,
}

/// A channel row and what stops it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChannelProblems {
    pub row: u32,
    pub problems: Vec<ChannelProblem>,
}

/// Whether `value` is a DMX universe, address or offset: a whole number
/// from 1 to 65535.
fn in_range(value: f64) -> bool {
    value.fract() == 0.0 && (1.0..=f64::from(u16::MAX)).contains(&value)
}

/// How many rows have each name, trimmed; a blank name is none.
fn name_counts<'a>(names: impl Iterator<Item = &'a str>) -> HashMap<&'a str, usize> {
    let mut counts = HashMap::new();
    for name in names.map(str::trim).filter(|n| !n.is_empty()) {
        *counts.entry(name).or_insert(0) += 1;
    }
    counts
}

/// What stops each fixture row, the rows with none left out. A fixture
/// whose type `from_gdtf` must name its mode.
pub fn venue_row_problems(rows: &[VenueRow], from_gdtf: impl Fn(&str) -> bool) -> Vec<RowProblems> {
    let counts = name_counts(rows.iter().map(|r| r.name.as_str()));
    let mut out = Vec::new();
    for (row, fixture) in (0u32..).zip(rows) {
        let mut problems = Vec::new();
        let name = fixture.name.trim();
        if name.is_empty() {
            problems.push(RowProblem::NoName);
        } else if counts[name] > 1 {
            problems.push(RowProblem::DuplicateName);
        }
        let fixture_type = fixture.fixture_type.trim();
        if fixture_type.is_empty() {
            problems.push(RowProblem::NoType);
        } else if from_gdtf(fixture_type) && fixture.mode.as_deref().is_none_or(str::is_empty) {
            problems.push(RowProblem::NoMode);
        }
        if !in_range(fixture.universe) {
            problems.push(RowProblem::BadUniverse);
        }
        if !in_range(fixture.start_channel) {
            problems.push(RowProblem::BadAddress);
        }
        if !problems.is_empty() {
            out.push(RowProblems { row, problems });
        }
    }
    out
}

/// What stops each channel row, the rows with none left out: two rows of
/// one name would otherwise save as one.
pub fn channel_row_problems(rows: &[ChannelRow]) -> Vec<ChannelProblems> {
    let counts = name_counts(rows.iter().map(|r| r.name.as_str()));
    let mut out = Vec::new();
    for (row, channel) in (0u32..).zip(rows) {
        let mut problems = Vec::new();
        let name = channel.name.trim();
        if name.is_empty() {
            problems.push(ChannelProblem::NoName);
        } else if counts[name] > 1 {
            problems.push(ChannelProblem::DuplicateName);
        }
        if !in_range(channel.offset) {
            problems.push(ChannelProblem::BadOffset);
        }
        if !problems.is_empty() {
            out.push(ChannelProblems { row, problems });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, fixture_type: &str, universe: f64, start_channel: f64) -> VenueRow {
        VenueRow {
            name: name.to_string(),
            fixture_type: fixture_type.to_string(),
            universe,
            start_channel,
            mode: None,
        }
    }

    #[test]
    fn rows_that_cannot_be_saved_are_named() {
        use RowProblem::*;
        let rows = [
            row("A", "par", 1.0, 1.0),
            row("", "par", 1.0, 1.0),
            row(" A", "par", 1.0, 1.0),
            row("B", "", 1.0, 1.0),
            row("C", "par", 0.0, 0.0),
            row("D", "par", 70000.0, 1.5),
            row("E", "gdtf", 1.0, 1.0),
        ];
        let found = venue_row_problems(&rows, |t| t == "gdtf");
        let by_row: Vec<(u32, Vec<RowProblem>)> =
            found.into_iter().map(|p| (p.row, p.problems)).collect();
        assert_eq!(
            by_row,
            vec![
                (0, vec![DuplicateName]),
                (1, vec![NoName]),
                (2, vec![DuplicateName]),
                (3, vec![NoType]),
                (4, vec![BadUniverse, BadAddress]),
                (5, vec![BadUniverse, BadAddress]),
                (6, vec![NoMode]),
            ]
        );
    }

    #[test]
    fn channel_rows_that_cannot_be_saved_are_named() {
        use ChannelProblem::*;
        let channel = |name: &str, offset| ChannelRow {
            name: name.to_string(),
            offset,
        };
        let found =
            channel_row_problems(&[channel("red", 1.0), channel("red ", 2.0), channel(" ", 0.0)]);
        let by_row: Vec<(u32, Vec<ChannelProblem>)> =
            found.into_iter().map(|p| (p.row, p.problems)).collect();
        assert_eq!(
            by_row,
            vec![
                (0, vec![DuplicateName]),
                (1, vec![DuplicateName]),
                (2, vec![NoName, BadOffset])
            ]
        );
    }
}
