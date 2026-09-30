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
 * Aiming a fixed fixture: the rotation that makes it face a direction or
 * point at a place. Pure math, degrees, rounded to a tenth. The convention is
 * the docs' "Mounting and pose convention": rotation is degrees about X, Y, Z
 * applied in that order; unrotated, a fixture points straight down (-Z).
 * Stage space is meters, +x stage-left, +y upstage, +z up.
 */

export type Vec3 = [number, number, number];

/** Bearings that face across the deck: the beam's horizontal heading, where
 *  0 is upstage (+y), -90 is stage-left (+x), 90 is stage-right (-x) and 180
 *  is downstage (-y). */
export const BEARINGS = {
  stageLeft: -90,
  stageRight: 90,
  upstage: 0,
  downstage: 180,
} as const;

/** Rounds to a tenth of a degree, never returning negative zero. */
export function round1(v: number): number {
  const r = Math.round(v * 10) / 10;
  return r === 0 ? 0 : r;
}

/**
 * The rotation that faces `bearingDeg` across the deck, tipped `tiltUpDeg`
 * up from the floor (0 is level, 90 straight up, negative aims down):
 * `(90 + tilt, 0, bearing)`. Unrotated points down, so 90 about X is level.
 */
export function faceRotation(bearingDeg: number, tiltUpDeg: number): Vec3 {
  return [round1(90 + tiltUpDeg), 0, round1(bearingDeg)];
}

/**
 * The rotation that points a fixed fixture's rest beam from `position` at
 * `target`: `a = acos(-d.z)` about X, `b = atan2(-d.x, d.y)` about Z, with
 * `d` the unit vector from the fixture to the target. Null when the fixture
 * is at the target and there is no direction to take.
 */
export function aimRotation(position: Vec3, target: Vec3): Vec3 | null {
  const dx = target[0] - position[0];
  const dy = target[1] - position[1];
  const dz = target[2] - position[2];
  const len = Math.hypot(dx, dy, dz);
  if (len < 1e-9) return null;
  const a = (Math.acos(Math.max(-1, Math.min(1, -dz / len))) * 180) / Math.PI;
  const b = (Math.atan2(-dx, dy) * 180) / Math.PI;
  return [round1(a), 0, round1(b)];
}

/**
 * Where a fixed fixture's rest beam points in stage space: the unit vector
 * `Rz(z) · Ry(y) · Rx(x) · (0, 0, -1)`. The plan draws the beam from it.
 */
export function restAim(rotation: Vec3): Vec3 {
  const [rx, ry, rz] = rotation.map((d) => (d * Math.PI) / 180);
  let [x, y, z] = [0, 0, -1];
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

/** Where a beam from `position` along `aim` meets the deck, if it points down. */
export function deckFootprint(
  position: Vec3,
  aim: Vec3,
): [number, number] | null {
  if (aim[2] >= -1e-4) return null;
  const t = position[2] / -aim[2];
  return [position[0] + aim[0] * t, position[1] + aim[1] * t];
}

/** The two mountings the docs name for a mover (aiming one is the show's job). */
export const MOVER_MOUNTINGS = {
  hung: [0, 0, 180] as Vec3,
  standing: [180, 0, 0] as Vec3,
};
