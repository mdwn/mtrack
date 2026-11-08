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
//! The patch export answers the venue editor's questions by the player's
//! own `patch::overlaps_with`: the cases the web UI's old TypeScript copy
//! was pinned by (`lighting-patch-math.spec.ts`), now against the rule, and
//! every overlap the loader reports for a whole venue (`venue_overlaps`)
//! is one the editor would have refused for that fixture.

use mtrack_rules_wasm::patch::{overlaps_with, venue_overlaps};
use mtrack_rules_wasm::PatchSpan;

fn span(fixture: &str, address: u16, footprint: u16, universe: u16) -> PatchSpan {
    PatchSpan {
        fixture: fixture.to_string(),
        universe,
        address,
        footprint,
    }
}

fn hit(o: &mtrack_rules_wasm::PatchOverlap) -> (Vec<&str>, u32, u32) {
    (
        o.second_gang.iter().map(String::as_str).collect(),
        o.from,
        o.to,
    )
}

fn bricks() -> Vec<PatchSpan> {
    (0..8)
        .map(|i| span(&format!("Brick{}", i + 1), 1 + i * 4, 4, 1))
        .collect()
}

#[test]
fn a_wider_mode_collides_with_the_next_fixture_by_name_and_addresses() {
    let hits = overlaps_with(&bricks(), &span("Brick3", 9, 7, 1));
    assert_eq!(
        hits.iter().map(hit).collect::<Vec<_>>(),
        vec![(vec!["Brick4"], 13, 15)]
    );
    assert!(overlaps_with(&bricks(), &span("Brick3", 9, 4, 1)).is_empty());
    assert!(overlaps_with(&bricks(), &span("Brick3", 9, 3, 1)).is_empty());
}

#[test]
fn a_gang_is_not_a_collision_and_an_unknown_footprint_is_never_guessed() {
    // The UI hands a footprint it does not know over as 0: nothing occupied.
    let spans = [span("A", 1, 4, 1), span("B", 1, 4, 1), span("C", 3, 0, 1)];
    assert!(overlaps_with(&spans, &span("A", 1, 4, 1)).is_empty());
    // Wider than its gang mate: now it is an overlap.
    let hits = overlaps_with(&spans, &span("A", 1, 5, 1));
    assert_eq!(
        hits.iter().map(hit).collect::<Vec<_>>(),
        vec![(vec!["B"], 1, 4)]
    );
    // Over a gang: one hit naming every member.
    let hits = overlaps_with(&spans, &span("D", 3, 1, 1));
    assert_eq!(
        hits.iter().map(hit).collect::<Vec<_>>(),
        vec![(vec!["A", "B"], 3, 3)]
    );
    // Another universe never collides.
    assert!(overlaps_with(&spans, &span("E", 1, 4, 2)).is_empty());
}

#[test]
fn hits_at_one_address_are_by_byte_order_of_the_gangs_first_name() {
    let spans = [
        span("a", 1, 4, 1),
        span("B", 1, 6, 1),
        span("ä", 2, 2, 1),
        span("Z", 2, 3, 1),
    ];
    let hits = overlaps_with(&spans, &span("New", 3, 1, 1));
    assert_eq!(
        hits.iter().map(|o| o.second.as_str()).collect::<Vec<_>>(),
        vec!["B", "Z", "a", "ä"]
    );
}

/// A small deterministic generator (no `rand` here): a venue of up to ten
/// fixtures with names that repeat, universes 1-2, addresses around and
/// past the universe end, footprints from none to twelve.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }

    fn venue(&mut self) -> Vec<PatchSpan> {
        let count = 1 + self.next(10) as usize;
        (0..count)
            .map(|i| {
                let fixture = if self.next(4) == 0 {
                    ["a", "B", "par", "Par", "x1", "x10"][self.next(6) as usize].to_string()
                } else {
                    format!("F{i}")
                };
                let address = if self.next(20) == 0 {
                    [0, 511, 512, 513, 600][self.next(5) as usize]
                } else {
                    1 + self.next(40) as u16
                };
                let footprint = if self.next(8) == 0 {
                    0
                } else {
                    1 + self.next(12) as u16
                };
                span(&fixture, address, footprint, 1 + self.next(2) as u16)
            })
            .collect()
    }
}

#[test]
fn every_venue_overlap_is_one_the_editor_would_refuse() {
    let mut lcg = Lcg(7);
    let mut seen = 0;
    for _ in 0..2000 {
        let spans = lcg.venue();
        for overlap in venue_overlaps(&spans) {
            seen += 1;
            // The loader names the first gang by its first member; asking
            // for that fixture at its own span (as the editor does before a
            // save) must name the second gang over the same addresses.
            let mine = spans
                .iter()
                .find(|s| s.fixture == overlap.first)
                .expect("the overlap names a fixture of the venue");
            // Same-named fixtures are one fixture to the editor (the one
            // being moved is skipped by name): only unique names are asked.
            if spans.iter().filter(|s| s.fixture == overlap.first).count() > 1 {
                continue;
            }
            let hits = overlaps_with(&spans, mine);
            assert!(
                hits.iter().any(|h| h.second_gang == overlap.second_gang
                    && h.universe == overlap.universe
                    && (h.from, h.to) == (overlap.from, overlap.to)),
                "{overlap:?} not among {hits:?} for {spans:?}"
            );
        }
    }
    assert!(seen > 1000, "only {seen} overlaps generated");
}
