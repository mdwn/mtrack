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
import { create } from "@bufbuild/protobuf";
import {
  StageFixtureSchema,
  StageLayoutSchema,
  StageVec3Schema,
} from "../../gen/player/v1/player_pb";
import { f32, layoutMetadata } from "./feed";

describe("f32", () => {
  it("gives back the decimal a person wrote, not the float's exact value", () => {
    for (const written of [2.7, 0.1, 12.3, -4.25, 3.5, 180, 0, 0.05, 1234.5]) {
      expect(f32(Math.fround(written))).toBe(written);
    }
  });

  it("leaves what is not a finite fraction alone", () => {
    expect(f32(Number.NaN)).toBeNaN();
    expect(f32(Infinity)).toBe(Infinity);
    expect(f32(7)).toBe(7);
  });

  it("is a number that is the same float", () => {
    for (const x of [1 / 3, Math.PI, 1e-7, 123456.79]) {
      const narrowed = Math.fround(x);
      expect(Math.fround(f32(narrowed))).toBe(narrowed);
    }
  });
});

describe("layoutMetadata", () => {
  it("reads geometry as the numbers the venue file holds, as far as 32 bits go", () => {
    const vec = (x: number, y: number, z: number) =>
      create(StageVec3Schema, { x, y, z });
    const layout = create(StageLayoutSchema, {
      id: 1,
      venue: { name: "v", focusPoints: { drummer: vec(0.1, 2.8, 1.4) } },
      fixtures: [
        create(StageFixtureSchema, {
          name: "a",
          type: "Par",
          position: vec(-2, 3.5, 4.2),
          beamAngle: 12.3,
        }),
      ],
    });
    const metadata = layoutMetadata(layout);
    expect(metadata.fixtures.a.position).toEqual([-2, 3.5, 4.2]);
    expect(metadata.fixtures.a.beam_angle).toBe(12.3);
    expect(metadata.venue?.focus_points.drummer).toEqual([0.1, 2.8, 1.4]);
    expect(metadata.venue).not.toHaveProperty("dir");
  });
});
