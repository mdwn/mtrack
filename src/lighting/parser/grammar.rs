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

use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "src/lighting/grammar.pest"]
pub struct LightingParser;

/// The items a parsed file holds -- its includes, fixture types, venues,
/// shows, sequences and tempo blocks -- out of the `file` rule's pair.
pub(super) fn items<'i>(
    pairs: pest::iterators::Pairs<'i, Rule>,
) -> impl Iterator<Item = pest::iterators::Pair<'i, Rule>> {
    pairs.flat_map(|p| {
        if p.as_rule() == Rule::file {
            p.into_inner().collect::<Vec<_>>()
        } else {
            vec![p]
        }
    })
}
