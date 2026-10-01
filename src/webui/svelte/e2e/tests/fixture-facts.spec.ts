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

// The fixture facts a GDTF type's page quotes, checked as plain values (no
// browser): what is stated is shown without noise, what is not is absent.

import { test, expect } from "@playwright/test";
import {
  beamFacts,
  outputFacts,
  powerFact,
  trimNumber,
  modeCounts,
  unresolvedFixtures,
  venueUse,
} from "../../src/lib/lighting/fixtureFacts";

const beam = (over: Record<string, unknown> = {}) => ({
  type: null,
  beam_angle: null,
  field_angle: null,
  luminous_flux: null,
  color_temperature: null,
  power: null,
  ...over,
});

test("numbers lose their trailing zeros", () => {
  expect(trimNumber(13)).toBe("13");
  expect(trimNumber(12.5)).toBe("12.5");
  expect(trimNumber(0.4000001)).toBe("0.4");
  expect(trimNumber(5500.0)).toBe("5500");
});

test("a fact the archive does not state is absent, never 0", () => {
  expect(beamFacts(null)).toBeNull();
  expect(beamFacts(beam())).toBeNull();
  expect(beamFacts(beam({ type: "  " }))).toBeNull();
  expect(beamFacts(beam({ type: "Wash", beam_angle: 13 }))).toEqual({
    type: "Wash",
    beam: "13",
  });
  expect(beamFacts(beam({ field_angle: 0 }))).toEqual({ field: "0" });
  expect(outputFacts(beam())).toBeNull();
  expect(outputFacts(beam({ color_temperature: 5500 }))).toEqual({
    cct: "5500",
  });
  expect(powerFact(beam())).toBeNull();
  expect(powerFact(beam({ power: 12 }))).toBe("12");
});

test("a long list of users collapses to a count, and a mode lists its own", () => {
  const names = (n: number, mode: string | null = "m") =>
    Array.from({ length: n }, (_, i) => ({ name: `B${i}`, mode }));
  const just = (n: number) => names(n).map((f) => f.name);
  const venues = [
    { name: "a", fixtures: names(8) },
    { name: "b", fixtures: names(9) },
    { name: "c", fixtures: [] },
  ];
  expect(venueUse(venues)).toEqual([
    { venue: "a", count: 8, names: just(8) },
    { venue: "b", count: 9, names: null },
  ]);
  const mixed = [
    {
      name: "v",
      fixtures: [
        { name: "X", mode: "m" },
        { name: "Y", mode: "n" },
        { name: "Z", mode: null },
      ],
    },
  ];
  expect(venueUse(mixed, "n")).toEqual([
    { venue: "v", count: 1, names: ["Y"] },
  ]);
  expect(venueUse(mixed, "q")).toEqual([]);
  expect([...modeCounts(mixed)]).toEqual([
    ["m", 1],
    ["n", 1],
  ]);
  expect(unresolvedFixtures(mixed)).toEqual([{ venue: "v", fixture: "Z" }]);
});
