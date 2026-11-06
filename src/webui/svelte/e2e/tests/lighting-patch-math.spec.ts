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

// The venue editor's placement of a new row and its row checks, as plain
// values (no browser), through the player's rules module loaded under
// node. The patch rule itself (what overlaps, what gangs) and the row
// checks (`lighting/venue_rows.rs`) are tested where they live.

import { test, expect } from "@playwright/test";
import { nextFixtureName, nextPatch } from "../../src/lib/lighting/venueRows";
import { rules } from "../../src/lib/rules";
import { loadRules } from "./rules-node";

test.beforeAll(loadRules);

test("a new fixture continues the patch: 1 → 4 → 8", () => {
  // Fixture 1 has 3 channels at 1: the next starts at 4. That one has 4
  // channels: the third starts at 8.
  const second = nextPatch({ universe: 1, address: 1, footprint: 3 }, 4);
  expect(second).toEqual({ universe: 1, address: 4 });
  const third = nextPatch({ ...second, footprint: 4 }, 4);
  expect(third).toEqual({ universe: 1, address: 8 });
  // The first fixture of a venue.
  expect(nextPatch(null, 7)).toEqual({ universe: 1, address: 1 });
});

test("a new fixture that would run past 512 starts the next universe", () => {
  expect(nextPatch({ universe: 2, address: 505, footprint: 4 }, 4)).toEqual({
    universe: 2,
    address: 509,
  });
  expect(nextPatch({ universe: 2, address: 505, footprint: 4 }, 5)).toEqual({
    universe: 3,
    address: 1,
  });
});

test("an unknown footprint steps by one", () => {
  expect(
    nextPatch({ universe: 1, address: 10, footprint: null }, null),
  ).toEqual({ universe: 1, address: 11 });
});

test("a new row's name is the first unused Fixture N", () => {
  expect(nextFixtureName([])).toBe("Fixture 1");
  expect(nextFixtureName(["Fixture 1", "Fixture 3"])).toBe("Fixture 2");
  expect(nextFixtureName([" Fixture 1 ", "Fixture 2"])).toBe("Fixture 3");
});

test("rows that cannot be saved are named, never dropped", () => {
  const row = (
    name: string,
    fixture_type = "par",
    universe = 1,
    start_channel = 1,
  ) => ({
    name,
    fixture_type,
    universe,
    start_channel,
  });
  const byRow = (found: { row: number; problems: string[] }[] | undefined) =>
    (found ?? []).map((p) => [p.row, p.problems]);
  const found = rules.venue_row_problems(
    [row("A"), row(""), row(" A"), row("B", ""), row("C", "par", 0, 0)],
    [],
  );
  // ` A` and `A` are one name, as the save has it.
  expect(byRow(found)).toEqual([
    [0, ["duplicateName"]],
    [1, ["noName"]],
    [2, ["duplicateName"]],
    [3, ["noType"]],
    [4, ["badUniverse", "badAddress"]],
  ]);
  // Past any universe the player addresses.
  expect(byRow(rules.venue_row_problems([row("D", "par", 70000)], []))).toEqual(
    [[0, ["badUniverse"]]],
  );
  expect(
    byRow(
      rules.channel_row_problems([
        { name: "red", offset: 1 },
        { name: "red", offset: 2 },
        { name: " ", offset: 0 },
      ]),
    ),
  ).toEqual([
    [0, ["duplicateName"]],
    [1, ["duplicateName"]],
    [2, ["noName", "badOffset"]],
  ]);
});
