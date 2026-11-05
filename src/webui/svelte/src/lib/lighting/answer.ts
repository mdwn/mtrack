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
 * The lighting endpoints' answers are the server's generated types, but a 200
 * is not proof that the server answered: a proxy's HTML page, `{}` or `[]`
 * arrive as 200s too, and a page that assumes the shape throws on first use.
 * `readAnswer` refuses those with a `BadAnswerError`, which the page words
 * from i18n; past that, the shape is the generated type's.
 */

/** The server answered, but not with what this page reads. */
export class BadAnswerError extends Error {
  constructor(what: string) {
    super(`The server's answer for ${what} was not understood`);
    this.name = "BadAnswerError";
  }
}

/** Reads a fetch response as `what`'s answer: JSON, and an object carrying
 *  at least one of `keys` (unrelated JSON is not this endpoint's answer).
 *  Anything else throws `BadAnswerError`. */
export async function readAnswer<T extends object>(
  res: Response,
  what: string,
  keys: (keyof T & string)[],
): Promise<T> {
  const body: unknown = await res.json().catch(() => null);
  const isAnswer =
    typeof body === "object" &&
    body !== null &&
    !Array.isArray(body) &&
    keys.some((k) => k in body);
  if (!isAnswer) throw new BadAnswerError(what);
  return body as T;
}
