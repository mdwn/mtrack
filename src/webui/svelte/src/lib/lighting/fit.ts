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
 * The Fit shows page's data: the facts and suggestions `GetLightingFit`
 * reports (design/lighting_ui_design.md, section 9), and the small pure rules
 * the page applies to them.
 */

import type { FitGroup } from "../../gen/rules/mtrack_rules";

/** Groups in the order the page lists them: those finding no fixtures first,
 *  each half by name. */
export function orderGroups(groups: FitGroup[]): FitGroup[] {
  return [...groups].sort((a, b) => {
    const ea = a.fixtures.length === 0 ? 0 : 1;
    const eb = b.fixtures.length === 0 ? 0 : 1;
    return ea - eb || a.name.localeCompare(b.name);
  });
}

/** Whether two selections name the same fixtures. */
export function sameSet(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((n) => b.includes(n));
}

interface VenueFileFixture {
  name: string;
  tags: string[];
  [key: string]: unknown;
}

/** The venue's fixtures with `tags` added to each of `names`. Everything else
 *  about every fixture (position, rotation, patch) is carried through as it
 *  is, and tags a fixture already has stay where they are. */
export function withTags<F extends VenueFileFixture>(
  fixtures: F[],
  names: string[],
  tags: string[],
): F[] {
  return fixtures.map((f) => {
    if (!names.includes(f.name)) return f;
    const missing = tags.filter((t) => !f.tags.includes(t));
    return missing.length === 0 ? f : { ...f, tags: [...f.tags, ...missing] };
  });
}

/** The `ola_patch` command for one universe. The device and port belong to
 *  the user's rig, so they are left as placeholders. */
export function olaPatchLine(universe: number): string {
  return `ola_patch -d <device> -p <port> -u ${universe}`;
}
