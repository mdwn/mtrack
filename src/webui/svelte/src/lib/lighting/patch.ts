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
 * The venue inspector's patch check (venue-exchange design §21): whether a
 * fixture's candidate address or mode runs over another fixture's
 * addresses, asked before the save. The same rule as the server's
 * `lighting::patch`: fixtures on the identical span (same universe, start
 * and footprint) are ganged on purpose, not overlapping; only a partial
 * intersection is refused. A fixture whose footprint is not known is never
 * guessed at.
 */

/** The addresses one fixture occupies. */
export interface Span {
  fixture: string;
  universe: number;
  address: number;
  /** Null when the fixture's type or mode does not load. */
  footprint: number | null;
}

/** The last address a span uses, or null when it uses none. */
export function lastAddress(span: Span): number | null {
  return span.footprint && span.footprint > 0
    ? span.address + span.footprint - 1
    : null;
}

/** Whether two spans are ganged: the identical addresses. */
export function ganged(a: Span, b: Span): boolean {
  return (
    a.universe === b.universe &&
    a.address === b.address &&
    a.footprint === b.footprint &&
    (a.footprint ?? 0) > 0
  );
}

/** A fixture the candidate would overlap, and the addresses both use. */
export interface Collision {
  fixture: string;
  from: number;
  to: number;
}

/** The fixtures `candidate` would partly overlap, by address then name. The
 *  candidate's own span (same name) and the ones it gangs with are skipped. */
export function collisions(spans: Span[], candidate: Span): Collision[] {
  const end = lastAddress(candidate);
  if (end === null) return [];
  const out: Collision[] = [];
  for (const s of spans) {
    if (s.fixture === candidate.fixture || s.universe !== candidate.universe)
      continue;
    const last = lastAddress(s);
    if (last === null || ganged(s, candidate)) continue;
    const from = Math.max(s.address, candidate.address);
    const to = Math.min(last, end);
    if (from <= to) out.push({ fixture: s.fixture, from, to });
  }
  return out.sort(
    (a, b) => a.from - b.from || a.fixture.localeCompare(b.fixture),
  );
}

/** How many addresses fit at the candidate's start before another
 *  fixture's (ganged ones aside): the largest footprint that would not
 *  collide. Zero when the start itself is taken. */
export function room(spans: Span[], candidate: Span, limit = 512): number {
  let end = limit;
  for (const s of spans) {
    if (s.fixture === candidate.fixture || s.universe !== candidate.universe)
      continue;
    const last = lastAddress(s);
    if (last === null || ganged(s, candidate)) continue;
    if (s.address <= candidate.address && last >= candidate.address) return 0;
    if (s.address > candidate.address) end = Math.min(end, s.address - 1);
  }
  return Math.max(0, end - candidate.address + 1);
}

/** The addresses the strip shows: `size` of them around the fixture, inside
 *  1–512, so a long universe is never drawn whole. */
export function stripWindow(
  address: number,
  footprint: number,
  size = 32,
): { from: number; to: number } {
  const span = Math.max(1, footprint);
  let from = address - Math.max(0, Math.floor((size - span) / 2));
  from = Math.max(1, Math.min(from, 512 - size + 1));
  return { from, to: Math.min(512, from + size - 1) };
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
  spans: Span[],
  candidate: Span,
  from: number,
  to: number,
): StripCell[] {
  const end = lastAddress(candidate);
  const others = spans.filter(
    (s) =>
      s.fixture !== candidate.fixture &&
      s.universe === candidate.universe &&
      lastAddress(s) !== null,
  );
  const cells: StripCell[] = [];
  for (let address = from; address <= to; address++) {
    const on = others.filter(
      (s) => address >= s.address && address <= (lastAddress(s) as number),
    );
    const mine = end !== null && address >= candidate.address && address <= end;
    cells.push({
      address,
      owners: on.map((s) => s.fixture),
      mine,
      clash: mine && on.some((s) => !ganged(s, candidate)),
    });
  }
  return cells;
}
