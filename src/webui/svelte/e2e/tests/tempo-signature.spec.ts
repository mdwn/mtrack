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

// The song editors' time signature at a bar, as plain values (no browser):
// the player's, through its tempo map.

import { test, expect } from "@playwright/test";
import { loadTempoMap } from "../../src/lib/tempo-wasm";
import type { TempoConfig } from "../../src/lib/api/songs";
import {
  beatsInMeasure,
  parseTimeSignature,
  sigAtMeasure,
} from "../../src/lib/util/tempo";

// The tempo map runs as WebAssembly, loaded once before any of these.
test.beforeAll(loadTempoMap);

test("a signature change mid-bar governs from the next downbeat (#628)", () => {
  const tempo: TempoConfig = {
    bpm: 120,
    time_signature: "4/4",
    changes: [
      { measure: 3, beat: 3, time_signature: "3/4" },
      { measure: 6, time_signature: "7/8" },
    ],
  };
  expect(sigAtMeasure(tempo, 2)).toEqual([4, 4]);
  // Written on bar 3's third beat: bar 3 keeps its four.
  expect(sigAtMeasure(tempo, 3)).toEqual([4, 4]);
  expect(sigAtMeasure(tempo, 4)).toEqual([3, 4]);
  expect(beatsInMeasure(tempo, 4)).toBe(3);
  // Written on a downbeat: that bar.
  expect(sigAtMeasure(tempo, 5)).toEqual([3, 4]);
  expect(sigAtMeasure(tempo, 6)).toEqual([7, 8]);
});

test("an edit to the tempo in place is seen at once", () => {
  const tempo: TempoConfig = { bpm: 120, time_signature: "4/4" };
  expect(sigAtMeasure(tempo, 2)).toEqual([4, 4]);
  tempo.changes = [{ measure: 2, time_signature: "5/4" }];
  expect(sigAtMeasure(tempo, 2)).toEqual([5, 4]);
  expect(sigAtMeasure(null, 9)).toEqual([4, 4]);
});

test("a time signature reads as written, 4/4 when it does not parse", () => {
  expect(parseTimeSignature(" 6 / 8 ")).toEqual([6, 8]);
  expect(parseTimeSignature("six-eight")).toEqual([4, 4]);
  expect(parseTimeSignature(undefined)).toEqual([4, 4]);
});
