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
 * The venue editor's rows: a name for a new one, and what stops a save. A
 * row the user added is never dropped without a word — a row that cannot be
 * saved blocks the save and says why.
 */

/** The first `Fixture N` (N from 1) no row is called. */
export function nextFixtureName(names: string[], base = "Fixture"): string {
  const taken = new Set(names.map((n) => n.trim()));
  for (let n = 1; ; n++) {
    const name = `${base} ${n}`;
    if (!taken.has(name)) return name;
  }
}

/** Why a row cannot be saved, as the suffix of a
 *  `lighting.venueRow.*` message. */
export type RowProblem =
  | "noName"
  | "duplicateName"
  | "noType"
  | "noMode"
  | "badUniverse"
  | "badAddress";

export interface Row {
  name: string;
  fixture_type: string;
  universe: number;
  start_channel: number;
  mode?: string | null;
}

const positiveInteger = (v: unknown) =>
  typeof v === "number" && Number.isInteger(v) && v >= 1;

/** Each row's problems, by index; a row with none is absent. A fixture from
 *  a GDTF (`fromGdtf`) must name its mode. */
export function rowProblems(
  rows: Row[],
  fromGdtf: (type: string) => boolean = () => false,
): Map<number, RowProblem[]> {
  const counts = new Map<string, number>();
  for (const row of rows) {
    const name = row.name.trim();
    if (name) counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  const out = new Map<number, RowProblem[]>();
  rows.forEach((row, i) => {
    const problems: RowProblem[] = [];
    const name = row.name.trim();
    if (!name) problems.push("noName");
    else if ((counts.get(name) ?? 0) > 1) problems.push("duplicateName");
    if (!row.fixture_type.trim()) problems.push("noType");
    else if (fromGdtf(row.fixture_type.trim()) && !row.mode)
      problems.push("noMode");
    if (!positiveInteger(row.universe)) problems.push("badUniverse");
    if (!positiveInteger(row.start_channel)) problems.push("badAddress");
    if (problems.length > 0) out.set(i, problems);
  });
  return out;
}

/** Why a fixture type's channel row cannot be saved, as the suffix of a
 *  `lighting.channelRow.*` message. */
export type ChannelProblem = "noName" | "duplicateName" | "badOffset";

/** Each channel row's problems, by index; a row with none is absent. Two
 *  rows of one name would otherwise save as one. */
export function channelProblems(
  rows: { name: string; offset: number }[],
): Map<number, ChannelProblem[]> {
  const counts = new Map<string, number>();
  for (const row of rows) {
    const name = row.name.trim();
    if (name) counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  const out = new Map<number, ChannelProblem[]>();
  rows.forEach((row, i) => {
    const problems: ChannelProblem[] = [];
    const name = row.name.trim();
    if (!name) problems.push("noName");
    else if ((counts.get(name) ?? 0) > 1) problems.push("duplicateName");
    if (!positiveInteger(row.offset)) problems.push("badOffset");
    if (problems.length > 0) out.set(i, problems);
  });
  return out;
}
