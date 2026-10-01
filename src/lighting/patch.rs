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

/// Fixtures patched at exactly the same span — same universe, start and
/// footprint. Ganging (two pars on one address, placeholders at 1:1) is
/// deliberate and common: every member receives the same bytes, nothing is
/// overwritten, so a gang is never an overlap. A lone fixture is a gang of
/// one.
#[derive(Clone, Debug)]
struct Gang<'a> {
    span: &'a PatchSpan,
    /// Every member's name, sorted.
    members: Vec<String>,
}

/// The spans grouped into gangs, sorted by universe, address, footprint.
/// Zero-footprint spans occupy nothing and are left out.
fn gangs(spans: &[PatchSpan]) -> Vec<Gang<'_>> {
    let mut sorted: Vec<&PatchSpan> = spans.iter().filter(|s| s.footprint > 0).collect();
    sorted.sort_by(|a, b| {
        (a.universe, a.address, a.footprint, &a.fixture).cmp(&(
            b.universe,
            b.address,
            b.footprint,
            &b.fixture,
        ))
    });
    let mut out: Vec<Gang<'_>> = Vec::new();
    for span in sorted {
        match out.last_mut() {
            Some(gang)
                if (gang.span.universe, gang.span.address, gang.span.footprint)
                    == (span.universe, span.address, span.footprint) =>
            {
                gang.members.push(span.fixture.clone());
            }
            _ => out.push(Gang {
                span,
                members: vec![span.fixture.clone()],
            }),
        }
    }
    out
}

/// Two fixtures (or gangs of them) patched over part of each other's
/// addresses: each writes over the other.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PatchOverlap {
    /// The first member of the gang that starts first (by address, then
    /// footprint, then name).
    pub first: String,
    /// The first member of the other gang.
    pub second: String,
    /// Every fixture of the first gang, sorted; `[first]` for a lone one.
    pub first_gang: Vec<String>,
    /// Every fixture of the second gang, sorted.
    pub second_gang: Vec<String>,
    pub universe: u16,
    /// The first and last shared address, inclusive.
    pub from: u32,
    pub to: u32,
}

/// A gang's members for a message: `"A"`, or `"A" (ganged with "B", "C")`.
fn gang_display(members: &[String]) -> String {
    match members.split_first() {
        Some((first, [])) => format!("\"{first}\""),
        Some((first, rest)) => format!(
            "\"{first}\" (ganged with {})",
            rest.iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None => String::new(),
    }
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
            "fixtures {} and {} are both patched on universe {} at {addresses}; \
             each will overwrite the other",
            gang_display(&self.first_gang),
            gang_display(&self.second_gang),
            self.universe
        )
    }
}

fn overlap_between(a: &Gang<'_>, b: &Gang<'_>) -> Option<PatchOverlap> {
    let (from, to) = a.span.intersect(b.span)?;
    Some(PatchOverlap {
        first: a.members[0].clone(),
        second: b.members[0].clone(),
        first_gang: a.members.clone(),
        second_gang: b.members.clone(),
        universe: a.span.universe,
        from,
        to,
    })
}

/// Every pair of gangs of `spans` that partially overlaps, each pair once,
/// sorted by universe, then address. Fixtures at an identical span are a
/// gang and never reported; a fixture partially over a gang is reported
/// once, against the whole gang.
pub fn venue_overlaps(spans: &[PatchSpan]) -> Vec<PatchOverlap> {
    let gangs = gangs(spans);
    let mut out = Vec::new();
    for (i, a) in gangs.iter().enumerate() {
        for b in &gangs[i + 1..] {
            // Sorted by universe then start: past this universe, or past
            // a's end, nothing later can intersect a.
            if b.span.universe != a.span.universe {
                break;
            }
            if let Some(overlap) = overlap_between(a, b) {
                out.push(overlap);
            } else if a
                .span
                .last()
                .is_some_and(|last| u32::from(b.span.address) > last)
            {
                break;
            }
        }
    }
    out
}

/// The gangs of `spans` a candidate patch would partially overlap — for an
/// editor checking a fixture's new address or mode before it is saved. A
/// span with the candidate's own name is the fixture being moved, and is
/// skipped; spans identical to the candidate's would gang with it, and are
/// not in the way.
pub fn overlaps_with(spans: &[PatchSpan], candidate: &PatchSpan) -> Vec<PatchOverlap> {
    let others: Vec<PatchSpan> = spans
        .iter()
        .filter(|s| s.fixture != candidate.fixture)
        .filter(|s| {
            (s.universe, s.address, s.footprint)
                != (candidate.universe, candidate.address, candidate.footprint)
        })
        .cloned()
        .collect();
    let mine = Gang {
        span: candidate,
        members: vec![candidate.fixture.clone()],
    };
    let mut out: Vec<PatchOverlap> = gangs(&others)
        .iter()
        .filter_map(|gang| overlap_between(&mine, gang))
        .collect();
    out.sort_by(|a, b| (a.from, &a.second).cmp(&(b.from, &b.second)));
    out
}

/// The last DMX address of a universe.
pub const UNIVERSE_SIZE: u32 = 512;

/// A fixture whose footprint runs past the end of its universe: the
/// addresses past 512 do not exist, so its last channels are never sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PatchOverrun {
    pub fixture: String,
    pub universe: u16,
    pub address: u16,
    pub footprint: u16,
    /// The last address it would need, past 512.
    pub last: u32,
}

impl fmt::Display for PatchOverrun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fixture \"{}\" is patched on universe {} at {} and needs {} addresses, \
             to {}; a universe ends at {UNIVERSE_SIZE}, so its last channels are never sent",
            self.fixture, self.universe, self.address, self.footprint, self.last
        )
    }
}

/// Every span that runs past address 512, sorted by universe, address,
/// name.
pub fn venue_overruns(spans: &[PatchSpan]) -> Vec<PatchOverrun> {
    let mut out: Vec<PatchOverrun> = spans
        .iter()
        .filter_map(|s| {
            let last = s.last()?;
            (last > UNIVERSE_SIZE).then(|| PatchOverrun {
                fixture: s.fixture.clone(),
                universe: s.universe,
                address: s.address,
                footprint: s.footprint,
                last,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.universe, a.address, &a.fixture).cmp(&(b.universe, b.address, &b.fixture))
    });
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
                first_gang: vec!["A".to_string()],
                second_gang: vec!["B".to_string()],
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

    #[test]
    fn fixtures_at_an_identical_span_are_a_gang_not_an_overlap() {
        let spans = [
            span("P1", 1, 1, 4),
            span("P2", 1, 1, 4),
            span("P3", 1, 1, 4),
            span("Q", 1, 5, 4),
        ];
        assert!(venue_overlaps(&spans).is_empty());
        // A placeholder pile of many at 1:1 is still nothing.
        let pile: Vec<PatchSpan> = (0..50).map(|i| span(&format!("F{i}"), 1, 1, 8)).collect();
        assert!(venue_overlaps(&pile).is_empty());
        // Same start, different footprint: that is a partial overlap.
        let spans = [span("A", 1, 1, 4), span("B", 1, 1, 6)];
        assert_eq!(venue_overlaps(&spans).len(), 1);
    }

    #[test]
    fn a_fixture_over_a_gang_is_named_against_the_gang_once() {
        let spans = [
            span("Par2", 1, 1, 4),
            span("Par1", 1, 1, 4),
            span("Wide", 1, 3, 6),
        ];
        let overlaps = venue_overlaps(&spans);
        assert_eq!(overlaps.len(), 1, "{overlaps:?}");
        let o = &overlaps[0];
        assert_eq!(o.first_gang, vec!["Par1".to_string(), "Par2".to_string()]);
        assert_eq!(o.second_gang, vec!["Wide".to_string()]);
        assert_eq!((o.first.as_str(), o.second.as_str()), ("Par1", "Wide"));
        assert_eq!((o.from, o.to), (3, 4));
        let text = o.to_string();
        assert!(
            text.contains("\"Par1\" (ganged with \"Par2\") and \"Wide\""),
            "{text}"
        );
    }

    #[test]
    fn a_candidate_gangs_with_an_identical_span_and_hits_a_gang_once() {
        let spans = [span("P1", 1, 1, 4), span("P2", 1, 1, 4), span("X", 1, 9, 4)];
        assert!(overlaps_with(&spans, &span("New", 1, 1, 4)).is_empty());
        let hits = overlaps_with(&spans, &span("New", 1, 3, 4));
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].second_gang,
            vec!["P1".to_string(), "P2".to_string()]
        );
    }

    #[test]
    fn a_span_past_512_is_an_overrun() {
        let spans = [
            span("A", 1, 510, 8),
            span("B", 1, 505, 8),
            span("C", 1, 1, 0),
        ];
        let overruns = venue_overruns(&spans);
        assert_eq!(overruns.len(), 1);
        assert_eq!(overruns[0].fixture, "A");
        assert_eq!(overruns[0].last, 517);
        assert!(
            overruns[0].to_string().contains("to 517"),
            "{}",
            overruns[0]
        );
    }
}
