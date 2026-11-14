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

import { describe, expect, it } from "vitest";
import { get } from "svelte/store";
import {
  isLayerName,
  layersProblem,
  MAX_LAYERS,
  playerLayers,
  setPlayerLayers,
} from "./layers";

describe("layer names", () => {
  it("are identifiers", () => {
    for (const ok of ["a", "_a", "accents", "b-c", "D9", "layer_1"]) {
      expect(isLayerName(ok), ok).toBe(true);
    }
    for (const bad of ["", "1st", "has space", "a:b", "-a", "a.b"]) {
      expect(isLayerName(bad), bad).toBe(false);
    }
  });
});

describe("layersProblem", () => {
  it("accepts one to sixteen unique identifiers", () => {
    expect(layersProblem(["a"])).toBeNull();
    const sixteen = Array.from({ length: MAX_LAYERS }, (_, i) => `l${i}`);
    expect(layersProblem(sixteen)).toBeNull();
  });

  it("says why a list is not a set", () => {
    expect(layersProblem([])).toBe("empty");
    const many = Array.from({ length: MAX_LAYERS + 1 }, (_, i) => `l${i}`);
    expect(layersProblem(many)).toBe("tooMany");
    expect(layersProblem(["a", "b c"])).toBe("notIdentifier");
    expect(layersProblem(["a", "a"])).toBe("duplicate");
  });
});

describe("the player's layers", () => {
  it("are kept as reported, and an empty report leaves the defaults", () => {
    setPlayerLayers(["floor", "accents"]);
    expect(get(playerLayers)).toEqual(["floor", "accents"]);
    setPlayerLayers([]);
    expect(get(playerLayers)).toBeNull();
    setPlayerLayers(undefined);
    expect(get(playerLayers)).toBeNull();
  });
});
