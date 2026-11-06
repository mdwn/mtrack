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

/**
 * The venue editor's patch placement (venue-exchange design §21), over the
 * player's own patch rule: `rules.patch_overlaps` is `lighting::patch`'s
 * `overlaps_with`, compiled to WebAssembly, and decides what a candidate
 * address or mode runs over -- fixtures on the identical span gang, a
 * footprint of nothing (the UI hands an unknown one over as 0) occupies
 * nothing. What is here is placement only: how much fits, and the strip of
 * addresses around a fixture.
 */

import type { PatchOverlap, PatchSpan } from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

export type { PatchOverlap, PatchSpan };

/** The gangs `candidate` would partly overlap, by address then name, each
 *  naming every fixture of the gang. None for a span that is not one (an
 *  address or footprint that is not a whole number in 0–65535). */
export function overlaps(
  spans: PatchSpan[],
  candidate: PatchSpan,
): PatchOverlap[] {
  return rules.patch_overlaps(spans, candidate) ?? [];
}

/** One entry per fixture the hits name, with the addresses it shares, in
 *  the order the editor lists them: by the first shared address, then by
 *  name as the locale orders it. Display order only; the rule's own is by
 *  gang, in byte order of the gang's first name. */
export function listed(
  hits: PatchOverlap[],
): { fixture: string; from: number; to: number }[] {
  return hits
    .flatMap((h) =>
      h.second_gang.map((fixture) => ({ fixture, from: h.from, to: h.to })),
    )
    .sort((a, b) => a.from - b.from || a.fixture.localeCompare(b.fixture));
}

/** How many addresses fit at the candidate's start before another
 *  fixture's (ganged ones aside): the largest footprint that would not
 *  collide. Zero when the start itself is taken. */
export function room(
  spans: PatchSpan[],
  candidate: PatchSpan,
  limit = rules.limits().universe_size,
): number {
  if (overlaps(spans, candidate).some((h) => h.from === candidate.address))
    return 0;
  // Widened to the end of the universe, the candidate hits everything
  // after its start; the nearest bounds what fits.
  const probe = { ...candidate, footprint: limit - candidate.address + 1 };
  let end = limit;
  for (const h of overlaps(spans, probe))
    if (h.from > candidate.address) end = Math.min(end, h.from - 1);
  return Math.max(0, end - candidate.address + 1);
}

/** The addresses the strip shows: `size` of them around the fixture, inside
 *  the universe, so a long universe is never drawn whole. */
export function stripWindow(
  address: number,
  footprint: number,
  size = 32,
): { from: number; to: number } {
  const span = Math.max(1, footprint);
  let from = address - Math.max(0, Math.floor((size - span) / 2));
  const last = rules.limits().universe_size;
  from = Math.max(1, Math.min(from, last - size + 1));
  return { from, to: Math.min(last, from + size - 1) };
}

/** One address cell of the strip. */
export interface StripCell {
  address: number;
  /** The other fixtures on it. */
  owners: string[];
  /** The candidate uses it. */
  mine: boolean;
  /** The candidate and another fixture (not a gang mate) both use it. */
  clash: boolean;
}

export function stripCells(
  spans: PatchSpan[],
  candidate: PatchSpan,
  from: number,
  to: number,
): StripCell[] {
  const end = rules.patch_last(candidate);
  const others = spans
    .filter(
      (s) =>
        s.fixture !== candidate.fixture && s.universe === candidate.universe,
    )
    .map((s) => ({
      fixture: s.fixture,
      from: s.address,
      to: rules.patch_last(s),
    }))
    .filter(
      (s): s is { fixture: string; from: number; to: number } =>
        s.to !== undefined,
    );
  const clashes = overlaps(spans, candidate);
  const cells: StripCell[] = [];
  for (let address = from; address <= to; address++) {
    cells.push({
      address,
      owners: others
        .filter((s) => address >= s.from && address <= s.to)
        .map((s) => s.fixture),
      mine: end !== undefined && address >= candidate.address && address <= end,
      clash: clashes.some((h) => address >= h.from && address <= h.to),
    });
  }
  return cells;
}
