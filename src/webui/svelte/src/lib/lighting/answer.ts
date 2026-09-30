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
 * Guards for the lighting endpoints' answers. A 200 is not proof of the shape
 * the page reads: a proxy's HTML page, a server older than the UI, or `{}` all
 * arrive as 200s, and a page that assumes the shape throws on first use. Each
 * endpoint's guard lives beside its types and normalizes what it accepts
 * (arrays default to `[]`); anything else is a `BadAnswerError`, which the
 * page words from i18n.
 */

/** The server answered, but not with what this page reads. */
export class BadAnswerError extends Error {
  constructor(what: string) {
    super(`The server's answer for ${what} was not understood`);
    this.name = "BadAnswerError";
  }
}

export type Obj = Record<string, unknown>;

export function isObj(value: unknown): value is Obj {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Whether `value` is an object carrying at least one of `keys`: `{}` and
 *  unrelated JSON are objects too, but not an answer to this endpoint. */
export function hasAny(value: unknown, keys: string[]): value is Obj {
  return isObj(value) && keys.some((k) => k in value);
}

/** The array at `value`, or `[]`. */
export function arr<T = unknown>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

/** The object at `value`, or `{}`. */
export function obj(value: unknown): Obj {
  return isObj(value) ? value : {};
}

/** The object at `value`, or null. */
export function objOrNull(value: unknown): Obj | null {
  return isObj(value) ? value : null;
}

/** The finite number at `value`, or `fallback`. */
export function num(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

/** The string at `value`, or `fallback`. */
export function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

/** Reads a fetch response's JSON, or throws `BadAnswerError` when it is not
 *  JSON (an HTML page, an empty body). */
export async function readJson(res: Response, what: string): Promise<unknown> {
  try {
    return await res.json();
  } catch {
    throw new BadAnswerError(what);
  }
}
