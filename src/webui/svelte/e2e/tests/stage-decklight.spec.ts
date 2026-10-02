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

// The light the beams put on the deck, checked as plain numbers (no
// browser): the formula the 3D view's deck shader runs.

import { test, expect } from "@playwright/test";
import {
  beamSpread,
  deckColor,
  deckLight,
  DEFAULT_FIELD_RATIO,
  GAIN,
  LIGHT_FLOATS,
  MAX_DECK_LIGHTS,
  packDeckLights,
  type DeckLight,
} from "../../src/lib/stage/decklight";

const WHITE: [number, number, number] = [1, 1, 1];

/** A unit aim, `tilt` degrees up from level, pointing upstage (+y). */
function aimedUp(tilt: number): [number, number, number] {
  const t = (tilt * Math.PI) / 180;
  return [0, Math.cos(t), Math.sin(t)];
}

function light(over: Partial<DeckLight>): DeckLight {
  const spread = beamSpread({ angle_deg: 25, field_deg: null });
  return {
    origin: [0, 0, 4],
    aim: [0, 0, -1],
    ...spread,
    rgb: WHITE,
    share: 1,
    ...over,
  };
}

const level = (l: DeckLight, x: number, y: number) => deckLight(l, x, y)[0];

test.describe("deck light", () => {
  test("the reference beam straight down lights its centre to 1", () => {
    expect(level(light({}), 0, 0)).toBeCloseTo(1, 6);
  });

  test("a hung beam is a pool: full in the beam, gone past the field", () => {
    const l = light({});
    const at = (deg: number) => 4 * Math.tan((deg * Math.PI) / 180);
    // Inside the beam (12.5° half): bright. Past the field (20° half): only
    // the spill, a few percent.
    expect(level(l, at(8), 0)).toBeGreaterThan(0.8);
    expect(level(l, at(16), 0)).toBeLessThan(level(l, at(8), 0));
    expect(level(l, at(30), 0)).toBeLessThan(0.02);
    expect(level(l, at(30), 0)).toBeGreaterThan(0);
  });

  test("a narrow beam on the floor aimed up at the band leaves the floor dark", () => {
    const brick = light({
      origin: [0, 0, 0.1],
      aim: aimedUp(45),
      ...beamSpread({ angle_deg: 13, field_deg: null }),
    });
    // Two meters upstage of it, on the deck: nothing but spill.
    expect(deckColor(deckLight(brick, 0, 2))[0]).toBeLessThan(0.02);
  });

  test("the same fixture with a diffuser lights the floor in front of it", () => {
    const bare = light({
      origin: [0, 0, 0.1],
      aim: aimedUp(45),
      ...beamSpread({ angle_deg: 13, field_deg: null }),
    });
    const diffused = {
      ...bare,
      ...beamSpread({ angle_deg: 13, field_deg: null }, 90),
    };
    const near = deckColor(deckLight(diffused, 0, 1))[0];
    expect(near).toBeGreaterThan(0.1);
    expect(near).toBeGreaterThan(3 * deckColor(deckLight(bare, 0, 1))[0]);
    // In front of it, not behind it; and fading with distance.
    expect(level(diffused, 0, -1)).toBeLessThan(level(diffused, 0, 1) / 20);
    expect(level(diffused, 0, 3)).toBeLessThan(level(diffused, 0, 1));
    expect(level(diffused, 0, 3)).toBeGreaterThan(0);
  });

  test("a wider beam is thinner: the same light over more floor", () => {
    const narrow = light({});
    const wide = light({
      ...beamSpread({ angle_deg: 25, field_deg: null }, 60),
    });
    expect(level(wide, 0, 0)).toBeLessThan(level(narrow, 0, 0));
    expect(level(wide, 2, 0)).toBeGreaterThan(level(narrow, 2, 0));
  });

  test("light from under the deck, or with no height, does not land", () => {
    expect(level(light({ origin: [0, 0, 0] }), 1, 0)).toBe(0);
    expect(level(light({ origin: [0, 0, -1], aim: [0, 0, 1] }), 0, 0)).toBe(0);
  });

  test("the beam angle a venue fixture states replaces the rig's", () => {
    const rig = { angle_deg: 12, field_deg: 20 };
    expect(beamSpread(rig)).toEqual({ beamDeg: 12, fieldDeg: 20 });
    // The field keeps the rig's proportion.
    expect(beamSpread(rig, 60)).toEqual({ beamDeg: 60, fieldDeg: 100 });
    // A rig without a field angle gets the default proportion.
    expect(beamSpread({ angle_deg: 20, field_deg: null }).fieldDeg).toBeCloseTo(
      20 * DEFAULT_FIELD_RATIO,
    );
    // Never past a half-space, and a nonsense override is the rig's angle.
    expect(beamSpread(rig, 150).fieldDeg).toBe(180);
    for (const bad of [0, -5, 181, NaN, null, undefined])
      expect(beamSpread(rig, bad).beamDeg).toBe(12);
  });

  test("the drawn colour saturates without changing hue", () => {
    expect(deckColor([0, 0, 0])).toEqual([0, 0, 0]);
    const dim = deckColor([0.1, 0.05, 0]);
    const bright = deckColor([100, 50, 0]);
    expect(bright[0]).toBeLessThanOrEqual(GAIN);
    expect(bright[0]).toBeGreaterThan(dim[0]);
    expect(dim[1] / dim[0]).toBeCloseTo(0.5, 9);
    expect(bright[1] / bright[0]).toBeCloseTo(0.5, 9);
  });

  test("packing leaves out dark lights and keeps the strongest", () => {
    const out = new Float32Array(MAX_DECK_LIGHTS * LIGHT_FLOATS);
    expect(
      packDeckLights(
        [light({}), light({ rgb: [0, 0, 0] }), light({ origin: [0, 0, -1] })],
        out,
      ),
    ).toBe(1);
    // Origin and the cosine of the half beam angle; aim and the field's.
    expect([...out.slice(0, 3)]).toEqual([0, 0, 4]);
    expect(out[3]).toBeCloseTo(Math.cos((12.5 * Math.PI) / 180), 6);
    expect([...out.slice(4, 7)]).toEqual([0, 0, -1]);
    expect(out[7]).toBeLessThan(out[3]);

    const many = Array.from({ length: MAX_DECK_LIGHTS + 10 }, (_, i) =>
      light({ origin: [i, 0, 4], rgb: i < 10 ? [0.01, 0.01, 0.01] : WHITE }),
    );
    expect(packDeckLights(many, out)).toBe(MAX_DECK_LIGHTS);
    // The ten faint ones are the ones dropped.
    const xs = Array.from(
      { length: MAX_DECK_LIGHTS },
      (_, i) => out[i * LIGHT_FLOATS],
    );
    expect(Math.min(...xs)).toBe(10);
  });
});
