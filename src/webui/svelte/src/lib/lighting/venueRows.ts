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

import { rules } from "../rules";

/**
 * The venue editor's rows: a name and an address for a new one. What stops
 * a save is the player's check (`rules.venue_row_problems`).
 */

/** The first `Fixture N` (N from 1) no row is called. */
export function nextFixtureName(names: string[], base = "Fixture"): string {
  const taken = new Set(names.map((n) => n.trim()));
  for (let n = 1; ; n++) {
    const name = `${base} ${n}`;
    if (!taken.has(name)) return name;
  }
}

/** Where the editor puts a fixture added after `previous` (the last row,
 *  not the highest address): the same universe, straight after it — its
 *  address plus its footprint, or plus one when that is not known. A
 *  fixture that would run past its universe starts the next one. The first
 *  fixture of a venue goes at 1:1. */
export function nextPatch(
  previous: {
    universe: number;
    address: number;
    footprint: number | null;
  } | null,
  footprint: number | null,
): { universe: number; address: number } {
  if (!previous) return { universe: 1, address: 1 };
  const step =
    previous.footprint && previous.footprint > 0 ? previous.footprint : 1;
  const own = footprint && footprint > 0 ? footprint : 1;
  const address = previous.address + step;
  // Past its universe's end, by the patch's own rule (or past any address).
  const span = {
    fixture: "",
    universe: previous.universe,
    address,
    footprint: own,
  };
  if ((rules.patch_overruns([span])?.length ?? 1) > 0)
    return { universe: previous.universe + 1, address: 1 };
  return { universe: previous.universe, address };
}
