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

// The aim and arrange math, checked as plain numbers (no browser): the
// worked example from the docs' "Mounting and pose convention" and the house
// rig's measured positions.

import { test, expect } from "@playwright/test";
import {
  BEARINGS,
  aimRotation,
  deckFootprint,
  faceRotation,
  restAim,
} from "../../src/lib/stage/aim";
import {
  alignOnLine,
  dominantAxis,
  mirrorAcrossCentre,
  mirrorRotation,
  nudge,
  spaceEvenly,
  type Placed,
} from "../../src/lib/stage/arrange";

/** The house rig as measured: two bricks per side, four across the front. */
const HOUSE: Placed[] = [
  { name: "Brick1", position: [-5.3, 3.87, 0] },
  { name: "Brick2", position: [-5.39, 1.325, 0] },
  { name: "Brick3", position: [-3.69, 0.01, 0] },
  { name: "Brick4", position: [-1.0, 0.03, 0] },
  { name: "Brick5", position: [1.5, 0.0, 0] },
  { name: "Brick6", position: [3.69, 0.02, 0] },
  { name: "Brick7", position: [6.8, 1.325, 0] },
  { name: "Brick8", position: [6.91, 3.87, 0] },
];
const side = (names: string[]) => HOUSE.filter((p) => names.includes(p.name));
const byName = (items: Placed[], name: string) =>
  items.find((p) => p.name === name)!.position;

test.describe("aim math", () => {
  test("the docs' worked example: a brick at (0.09, -0.1, 0) aimed at (0.7, 1.9, 1.5)", () => {
    expect(aimRotation([0.09, -0.1, 0], [0.7, 1.9, 1.5])).toEqual([
      125.7, 0, -17.0,
    ]);
  });

  test("a fixture at the target has no aim", () => {
    expect(aimRotation([1, 2, 3], [1, 2, 3])).toBeNull();
  });

  test("straight down and straight up need no bearing", () => {
    expect(aimRotation([0, 0, 3], [0, 0, 0])).toEqual([0, 0, 0]);
    expect(aimRotation([0, 0, 0], [0, 0, 3])).toEqual([180, 0, 0]);
  });

  test("the perimeter rig faces in, tipped 20 degrees up", () => {
    expect(faceRotation(BEARINGS.stageLeft, 20)).toEqual([110, 0, -90]);
    expect(faceRotation(BEARINGS.upstage, 20)).toEqual([110, 0, 0]);
    expect(faceRotation(BEARINGS.stageRight, 20)).toEqual([110, 0, 90]);
    expect(faceRotation(BEARINGS.downstage, -30)).toEqual([60, 0, 180]);
  });

  test("a face rotation's beam heads where the bearing says", () => {
    const [x, y] = restAim(faceRotation(BEARINGS.stageLeft, 0));
    expect(x).toBeCloseTo(1, 9); // +x is stage-left
    expect(y).toBeCloseTo(0, 9);
    const up = restAim(faceRotation(BEARINGS.upstage, 20));
    expect(up[1]).toBeCloseTo(Math.cos((20 * Math.PI) / 180), 9);
    expect(up[2]).toBeCloseTo(Math.sin((20 * Math.PI) / 180), 9);
  });

  test("an aimed rotation's rest beam lands on the target", () => {
    const from: [number, number, number] = [0.09, -0.1, 0];
    const to: [number, number, number] = [0.7, 1.9, 1.5];
    const aim = restAim(aimRotation(from, to)!);
    const d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    const len = Math.hypot(...d);
    // Rounding to 0.1 degree keeps it within a centimeter or two at 2.5 m.
    for (let i = 0; i < 3; i++) expect(aim[i]).toBeCloseTo(d[i] / len, 2);
  });

  test("a hung mover's rest beam falls to the deck under it", () => {
    expect(deckFootprint([1, 2, 5], restAim([0, 0, 180]))).toEqual([1, 2]);
    expect(deckFootprint([1, 2, 5], restAim([90, 0, 0]))).toBeNull();
  });
});

test.describe("arrange math", () => {
  test("align on the measured sides puts them at x = -5.345 and 6.855", () => {
    const left = alignOnLine(side(["Brick1", "Brick2"]));
    const right = alignOnLine(side(["Brick7", "Brick8"]));
    expect(byName(left, "Brick1")).toEqual([-5.345, 3.87, 0]);
    expect(byName(left, "Brick2")).toEqual([-5.345, 1.325, 0]);
    expect(byName(right, "Brick7")).toEqual([6.855, 1.325, 0]);
    expect(byName(right, "Brick8")).toEqual([6.855, 3.87, 0]);
  });

  test("the dominant axis decides: a row aligns in y, z is untouched", () => {
    const row = side(["Brick3", "Brick4", "Brick5", "Brick6"]);
    expect(dominantAxis(row)).toBe("x");
    const out = alignOnLine(
      row.map((p) => ({ ...p, position: [...p.position] as never })),
    );
    const ys = new Set(out.map((p) => p.position[1]));
    expect(ys).toEqual(new Set([0.015]));
    expect(out.map((p) => p.position[0])).toEqual(
      row.map((p) => p.position[0]),
    );
  });

  test("space evenly on the front row makes 2.46 m gaps", () => {
    const row = side(["Brick3", "Brick4", "Brick5", "Brick6"]);
    const out = spaceEvenly(row);
    expect(out.map((p) => p.position[0])).toEqual([-3.69, -1.23, 1.23, 3.69]);
    // The other axes stay as measured.
    expect(out.map((p) => p.position[1])).toEqual(
      row.map((p) => p.position[1]),
    );
  });

  test("space evenly keeps the extremes and works in current order", () => {
    // Given out of order: the result is still spread by position, and the
    // returned list keeps the caller's order.
    const shuffled = [
      { name: "b", position: [3, 0, 0] },
      { name: "a", position: [0, 0, 0] },
      { name: "c", position: [10, 0, 0] },
      { name: "d", position: [4, 0, 0] },
    ] as Placed[];
    const out = spaceEvenly(shuffled);
    expect(out.map((p) => p.name)).toEqual(["b", "a", "c", "d"]);
    expect(byName(out, "a")[0]).toBe(0);
    expect(byName(out, "b")[0]).toBe(3.333);
    expect(byName(out, "d")[0]).toBe(6.667);
    expect(byName(out, "c")[0]).toBe(10);
  });

  test("space evenly needs three; two are left where they are", () => {
    const two = side(["Brick1", "Brick2"]);
    expect(spaceEvenly(two)).toEqual(two);
  });

  test("space evenly on a column runs along y", () => {
    const col: Placed[] = [
      { name: "a", position: [-6, 0, 0] },
      { name: "b", position: [-6, 1, 0] },
      { name: "c", position: [-6, 6, 0] },
    ];
    expect(spaceEvenly(col).map((p) => p.position[1])).toEqual([0, 3, 6]);
  });

  test("mirror about the centre negates x and nothing else", () => {
    const out = mirrorAcrossCentre(HOUSE);
    expect(byName(out, "Brick1")).toEqual([5.3, 3.87, 0]);
    expect(byName(out, "Brick7")).toEqual([-6.8, 1.325, 0]);
    // Mirroring twice is the identity; a fixture on the line stays put.
    expect(mirrorAcrossCentre(out)).toEqual(HOUSE);
    expect(
      mirrorAcrossCentre([{ name: "m", position: [0, 1, 2] }])[0].position,
    ).toEqual([0, 1, 2]);
  });

  test("mirror reflects the aim: (rx, -ry, -rz)", () => {
    const out = mirrorAcrossCentre([
      { name: "left", position: [-6.1, 3.87, 0], rotation: [110, 0, -90] },
      { name: "front", position: [-3.69, 0.01, 0], rotation: [110, 0, 0] },
      { name: "tilted", position: [1, 1, 0], rotation: [30, 15, 45] },
      { name: "bare", position: [2, 1, 0] },
    ]);
    expect(out[0].rotation).toEqual([110, 0, 90]);
    expect(out[1].rotation).toEqual([110, 0, 0]);
    expect(out[2].rotation).toEqual([30, -15, -45]);
    // A fixture with no rotation stays without one.
    expect("rotation" in out[3]).toBe(false);
    // Mirroring twice is the identity.
    expect(mirrorRotation(mirrorRotation([30, 15, 45]))).toEqual([30, 15, 45]);
  });

  test("a mirrored beam is the reflection of the original", () => {
    const rot: [number, number, number] = [110, 20, -70];
    const [x, y, z] = restAim(rot);
    const m = restAim(mirrorRotation(rot));
    expect(m[0]).toBeCloseTo(-x, 9);
    expect(m[1]).toBeCloseTo(y, 9);
    expect(m[2]).toBeCloseTo(z, 9);
  });

  test("nudge moves by the step without float dust", () => {
    const out = nudge([{ name: "n", position: [0.3, 1.1, 3] }], 0.1, -0.1);
    expect(out[0].position).toEqual([0.4, 1, 3]);
    expect(
      nudge([{ name: "n", position: [0, 0, 0] }], -1, 1)[0].position,
    ).toEqual([-1, 1, 0]);
  });
});
