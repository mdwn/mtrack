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

//! Patch overlap: two fixtures of one venue whose DMX addresses intersect on
//! the same universe. Both would be driven, each writing over the other, and
//! nothing on stage says why. Once a fixture's mode is its own choice
//! (venue-exchange design §21) so is its footprint, so a re-moded fixture can
//! grow into its neighbour's addresses without its line changing at all.
//!
//! Pure functions over [`PatchSpan`]s, so the loader, the readiness report
//! and an editor checking a candidate patch all ask the same question.

use std::fmt;

use serde::Serialize;

/// The addresses one fixture occupies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PatchSpan {
    /// The fixture's name.
    pub fixture: String,
    pub universe: u16,
    /// The first address, 1-based.
    pub address: u16,
    /// How many addresses it uses: the highest DMX offset its mode's
    /// expansion writes ([`crate::lighting::types::FixtureType::footprint`]).
    /// Zero occupies nothing.
    pub footprint: u16,
}

impl PatchSpan {
    /// The last address, inclusive; `None` for a zero footprint. Widened, so
    /// a fixture patched past 512 reports where it ends instead of wrapping.
    fn last(&self) -> Option<u32> {
        (self.footprint > 0).then(|| u32::from(self.address) + u32::from(self.footprint) - 1)
    }

    /// The addresses both spans use, inclusive, when they share a universe.
    fn intersect(&self, other: &PatchSpan) -> Option<(u32, u32)> {
        if self.universe != other.universe {
            return None;
        }
        let from = u32::from(self.address).max(u32::from(other.address));
        let to = self.last()?.min(other.last()?);
        (from <= to).then_some((from, to))
    }
}

/// Two fixtures patched over the same addresses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PatchOverlap {
    /// The fixture that starts first (by address, then name).
    pub first: String,
    pub second: String,
    pub universe: u16,
    /// The first and last shared address, inclusive.
    pub from: u32,
    pub to: u32,
}

impl fmt::Display for PatchOverlap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addresses = if self.from == self.to {
            format!("address {}", self.from)
        } else {
            format!("addresses {}-{}", self.from, self.to)
        };
        write!(
            f,
            "fixtures \"{}\" and \"{}\" are both patched on universe {} at {addresses}; \
             each will overwrite the other",
            self.first, self.second, self.universe
        )
    }
}

/// Every pair of `spans` that overlaps, each pair once, sorted by universe,
/// then address.
pub fn venue_overlaps(spans: &[PatchSpan]) -> Vec<PatchOverlap> {
    let mut sorted: Vec<&PatchSpan> = spans.iter().collect();
    sorted.sort_by(|a, b| {
        (a.universe, a.address, &a.fixture).cmp(&(b.universe, b.address, &b.fixture))
    });
    let mut out = Vec::new();
    for (i, a) in sorted.iter().enumerate() {
        for b in &sorted[i + 1..] {
            if let Some((from, to)) = a.intersect(b) {
                out.push(PatchOverlap {
                    first: a.fixture.clone(),
                    second: b.fixture.clone(),
                    universe: a.universe,
                    from,
                    to,
                });
            }
        }
    }
    out
}

/// The fixtures of `spans` a candidate patch would overlap — for an editor
/// checking a fixture's new address or mode before it is saved. A span
/// with the candidate's own name is the fixture being moved, and is skipped.
pub fn overlaps_with(spans: &[PatchSpan], candidate: &PatchSpan) -> Vec<PatchOverlap> {
    let mut out: Vec<PatchOverlap> = spans
        .iter()
        .filter(|s| s.fixture != candidate.fixture)
        .filter_map(|s| {
            candidate.intersect(s).map(|(from, to)| PatchOverlap {
                first: candidate.fixture.clone(),
                second: s.fixture.clone(),
                universe: candidate.universe,
                from,
                to,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.from, &a.second).cmp(&(b.from, &b.second)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(fixture: &str, universe: u16, address: u16, footprint: u16) -> PatchSpan {
        PatchSpan {
            fixture: fixture.to_string(),
            universe,
            address,
            footprint,
        }
    }

    #[test]
    fn adjacent_fixtures_do_not_overlap() {
        let spans = [span("A", 1, 1, 4), span("B", 1, 5, 4)];
        assert!(venue_overlaps(&spans).is_empty());
    }

    #[test]
    fn a_shared_address_is_reported_once_with_its_range() {
        let spans = [span("B", 1, 3, 4), span("A", 1, 1, 4), span("C", 2, 1, 4)];
        let overlaps = venue_overlaps(&spans);
        assert_eq!(
            overlaps,
            vec![PatchOverlap {
                first: "A".to_string(),
                second: "B".to_string(),
                universe: 1,
                from: 3,
                to: 4,
            }]
        );
        let text = overlaps[0].to_string();
        assert!(text.contains("\"A\" and \"B\""), "{text}");
        assert!(text.contains("universe 1 at addresses 3-4"), "{text}");
    }

    #[test]
    fn one_shared_address_reads_singular_and_containment_counts() {
        let spans = [
            span("A", 1, 1, 4),
            span("B", 1, 4, 1),
            span("Big", 1, 1, 10),
        ];
        let overlaps = venue_overlaps(&spans);
        assert_eq!(overlaps.len(), 3, "{overlaps:?}");
        let ab = overlaps
            .iter()
            .find(|o| o.first == "A" && o.second == "B")
            .unwrap();
        assert!(ab.to_string().contains("at address 4;"), "{ab}");
    }

    #[test]
    fn a_zero_footprint_occupies_nothing() {
        let spans = [span("A", 1, 1, 0), span("B", 1, 1, 4)];
        assert!(venue_overlaps(&spans).is_empty());
    }

    #[test]
    fn a_candidate_names_what_it_would_overlap_and_skips_itself() {
        let spans = [span("A", 1, 1, 4), span("B", 1, 9, 4), span("C", 1, 20, 1)];
        // Moving A to 7 with a wider mode: overlaps B, not C, not itself.
        let hits = overlaps_with(&spans, &span("A", 1, 7, 6));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].second, "B");
        assert_eq!((hits[0].from, hits[0].to), (9, 12));
        assert!(overlaps_with(&spans, &span("D", 2, 1, 512)).is_empty());
    }

    #[test]
    fn a_span_past_the_universe_end_does_not_wrap() {
        let spans = [span("A", 1, 510, 8), span("B", 1, 1, 4)];
        assert!(venue_overlaps(&spans).is_empty());
    }
}
