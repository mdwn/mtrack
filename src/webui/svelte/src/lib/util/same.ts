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
 * Whether two values of plain data (what JSON carries: objects, arrays,
 * strings, numbers, booleans, null) say the same thing, whatever order
 * their keys are in. A key that is undefined counts as absent.
 */
export function sameData(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || !a || !b) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((v, i) => sameData(v, b[i]));
  }
  const [ra, rb] = [a as Record<string, unknown>, b as Record<string, unknown>];
  const keys = (r: Record<string, unknown>) =>
    Object.keys(r).filter((k) => r[k] !== undefined);
  const ka = keys(ra);
  return (
    ka.length === keys(rb).length && ka.every((k) => sameData(ra[k], rb[k]))
  );
}
