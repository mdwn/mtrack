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

// The venue inspector's patch check, as plain values (no browser): the same
// rule as the server's lighting::patch — a gang is not an overlap.

import { test, expect } from "@playwright/test";
import {
  collisions,
  ganged,
  nextPatch,
  room,
  stripCells,
  stripWindow,
  type Span,
} from "../../src/lib/lighting/patch";
import {
  channelProblems,
  nextFixtureName,
  rowProblems,
} from "../../src/lib/lighting/venueRows";

const span = (
  fixture: string,
  address: number,
  footprint: number | null,
  universe = 1,
): Span => ({ fixture, universe, address, footprint });

const bricks = Array.from({ length: 8 }, (_, i) =>
  span(`Brick${i + 1}`, 1 + i * 4, 4),
);

test("a wider mode collides with the next fixture, by name and addresses", () => {
  const hits = collisions(bricks, span("Brick3", 9, 7));
  expect(hits).toEqual([{ fixture: "Brick4", from: 13, to: 15 }]);
  expect(collisions(bricks, span("Brick3", 9, 4))).toEqual([]);
  expect(collisions(bricks, span("Brick3", 9, 3))).toEqual([]);
});

test("a gang is not a collision, and an unknown footprint is never guessed", () => {
  const spans = [span("A", 1, 4), span("B", 1, 4), span("C", 3, null)];
  expect(ganged(spans[0], spans[1])).toBe(true);
  expect(collisions(spans, span("A", 1, 4))).toEqual([]);
  // Wider than its gang mate: now it is an overlap.
  expect(collisions(spans, span("A", 1, 5))).toEqual([
    { fixture: "B", from: 1, to: 4 },
  ]);
  expect(collisions(spans, span("D", 3, 1))).toEqual([
    { fixture: "A", from: 3, to: 3 },
    { fixture: "B", from: 3, to: 3 },
  ]);
  // Another universe never collides.
  expect(collisions(spans, span("E", 1, 4, 2))).toEqual([]);
});

test("room is how many addresses fit before the next fixture", () => {
  expect(room(bricks, span("Brick3", 9, 7))).toBe(4);
  expect(room(bricks, span("Brick8", 29, 7))).toBe(512 - 29 + 1);
  // Starting inside someone else's addresses leaves none.
  expect(room(bricks, span("X", 10, 1))).toBe(0);
});

test("the strip is a bounded window around the fixture", () => {
  expect(stripWindow(9, 4)).toEqual({ from: 1, to: 32 });
  expect(stripWindow(200, 4)).toEqual({ from: 186, to: 217 });
  expect(stripWindow(510, 4)).toEqual({ from: 481, to: 512 });
  const cells = stripCells(bricks, span("Brick3", 9, 7), 1, 32);
  expect(cells).toHaveLength(32);
  const at = (a: number) => cells.find((c) => c.address === a)!;
  expect(at(9)).toMatchObject({ mine: true, clash: false, owners: [] });
  expect(at(13)).toMatchObject({ mine: true, clash: true, owners: ["Brick4"] });
  expect(at(16)).toMatchObject({ mine: false, owners: ["Brick4"] });
});

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
  const found = rowProblems([
    row("A"),
    row(""),
    row("A"),
    row("B", ""),
    row("C", "par", 0, 0),
  ]);
  expect([...found]).toEqual([
    [0, ["duplicateName"]],
    [1, ["noName"]],
    [2, ["duplicateName"]],
    [3, ["noType"]],
    [4, ["badUniverse", "badAddress"]],
  ]);
  expect([
    ...channelProblems([
      { name: "red", offset: 1 },
      { name: "red", offset: 2 },
      { name: " ", offset: 0 },
    ]),
  ]).toEqual([
    [0, ["duplicateName"]],
    [1, ["duplicateName"]],
    [2, ["noName", "badOffset"]],
  ]);
});
