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

// The stage rig math, checked as plain numbers (no browser): the pose and
// mounting conventions the 3D view relies on, pinned against the Rust
// pointing math in src/lighting/effects/pointing.rs.

import { test, expect } from "@playwright/test";
import { Euler, Vector3 } from "three";
import {
  beamLength,
  beamLook,
  deckExtent,
  genericRig,
  isMover,
  poseRotations,
  trayPositions,
  type RigModel,
} from "../../src/lib/stage/rig";

/** pointing.rs `out_of_frame`: R = Rz·Ry·Rx applied to a vector. */
function outOfFrame(
  deg: [number, number, number],
  v: [number, number, number],
) {
  const [rx, ry, rz] = deg.map((d) => (d * Math.PI) / 180);
  let [x, y, z] = v;
  [y, z] = [
    Math.cos(rx) * y - Math.sin(rx) * z,
    Math.sin(rx) * y + Math.cos(rx) * z,
  ];
  [x, z] = [
    Math.cos(ry) * x + Math.sin(ry) * z,
    -Math.sin(ry) * x + Math.cos(ry) * z,
  ];
  [x, y] = [
    Math.cos(rz) * x - Math.sin(rz) * y,
    Math.sin(rz) * x + Math.cos(rz) * y,
  ];
  return [x, y, z];
}

const close = (a: number[], b: number[]) =>
  a.every((v, i) => Math.abs(v - b[i]) < 1e-9);

test.describe("stage rig math", () => {
  test("the venue rotation is three's ZYX Euler, matching pointing.rs", () => {
    for (const deg of [
      [90, 0, 90],
      [30, -45, 120],
      [0, 0, 180],
      [-20, 70, 15],
    ] as [number, number, number][]) {
      const euler = new Euler(
        (deg[0] * Math.PI) / 180,
        (deg[1] * Math.PI) / 180,
        (deg[2] * Math.PI) / 180,
        "ZYX",
      );
      for (const v of [
        [0, 1, 0],
        [1, 0, 0],
        [0.3, -0.4, 0.86],
      ] as [number, number, number][]) {
        const three = new Vector3(...v).applyEuler(euler);
        expect(close([three.x, three.y, three.z], outOfFrame(deg, v))).toBe(
          true,
        );
      }
    }
  });

  test("pan and tilt turn the axes the documented way", () => {
    const mover = genericRig("m");
    mover.tilt = 1;
    mover.pan = 0;
    expect(isMover(mover)).toBe(true);
    // Pose degrees are GDTF's, so the mapping is the identity in radians.
    const { panZ, tiltX } = poseRotations(mover, 30, -20);
    expect(panZ).toBeCloseTo((30 * Math.PI) / 180);
    expect(tiltX).toBeCloseTo((-20 * Math.PI) / 180);

    // The beam through the joints: R = Rz(pan)·Rx(tilt) applied to
    // (0, 0, −1). three's Euler order "ZYX" is that matrix, so X turns
    // the vector first.
    const beam = (pan: number, tilt: number) => {
      const r = poseRotations(mover, pan, tilt);
      const v = new Vector3(0, 0, -1).applyEuler(
        new Euler(r.tiltX, 0, r.panZ, "ZYX"),
      );
      return [v.x, v.y, v.z];
    };
    const s45 = Math.SQRT1_2;

    // At rest a hung head looks straight down, along −Z.
    expect(close(beam(0, 0), [0, 0, -1])).toBe(true);
    // Positive tilt swings the beam from −Z toward +Y.
    expect(close(beam(0, 45), [0, s45, -s45])).toBe(true);
    // Pan turns about the beam at rest, which leaves it where it is.
    expect(close(beam(90, 0), [0, 0, -1])).toBe(true);
    // Tilt 90 puts the beam on +Y; pan 90 then turns it counter-clockwise
    // seen from above, onto −X.
    expect(close(beam(90, 90), [-1, 0, 0])).toBe(true);

    // A rig without axes ignores the pose; nothing raises its root.
    const par: RigModel = genericRig("p");
    expect(isMover(par)).toBe(false);
    expect(poseRotations(par, 90, 90)).toEqual({ panZ: 0, tiltX: 0 });
  });

  test("the generic rig's lens looks straight down at rest", () => {
    const rig = genericRig("x");
    const lens = rig.nodes[1].transform;
    // Local −Z of the lens node in the body's frame is the third column
    // negated: (0, 0, −1), the GDTF rest direction.
    expect([-lens[0][2] + 0, -lens[1][2] + 0, -lens[2][2] + 0]).toEqual([
      0, 0, -1,
    ]);
    expect(rig.beams[0].node).toBe(1);
  });

  test("beams reach the deck or a fixed throw", () => {
    const style = { maxLength: 24, skyLength: 4 };
    expect(beamLength([0, 0, 4], [0, 0, -1], style)).toEqual({
      length: 4,
      onDeck: true,
    });
    const shallow = beamLength([0, 0, 4], [0, 0.999, -0.01], style);
    expect(shallow.onDeck).toBe(false);
    expect(shallow.length).toBe(24);
    expect(beamLength([0, 0, 4], [0, 1, 0], style)).toEqual({
      length: 4,
      onDeck: false,
    });
    expect(beamLength([0, 0, 4], [0, 0, 1], style).length).toBe(4);
  });

  test("channels become a colour and a level", () => {
    const red = beamLook({ red: 255, green: 0, blue: 0, dimmer: 128 });
    expect(red.rgb[0]).toBeCloseTo(128 / 255);
    expect(red.rgb[1]).toBe(0);
    expect(red.intensity).toBeCloseTo(128 / 255 / 3);
    expect(red.strobeOn).toBe(true);
    const warm = beamLook({ dimmer: 255 });
    expect(warm.rgb[0]).toBe(1);
    expect(warm.intensity).toBeGreaterThan(0.9);
    const dark = beamLook({ red: 0, green: 0, blue: 0 });
    expect(dark.intensity).toBe(0);
  });

  test("the tray and the deck extent hold everything", () => {
    const tray = trayPositions(["b", "a", "c"]);
    expect(tray.a[0]).toBeLessThan(tray.b[0]);
    expect(tray.b[0]).toBe(0);
    expect(tray.c[1]).toBeLessThan(0);
    expect(deckExtent([])).toEqual([-4, 4, 0, 6]);
    expect(deckExtent([[-6, 9, 3]])).toEqual([-7, 7, 0, 10]);
    // Something downstage of the audience edge pulls the deck down to it.
    expect(deckExtent([[0, -4.3, 2]])).toEqual([-4, 4, -5.3, 6]);
  });
});
