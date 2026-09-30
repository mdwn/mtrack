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
 * Arranging a selection on the plan: align, space, mirror, nudge. Pure
 * functions over `{name, position}` that return the new positions; the caller
 * writes them through the venue save. Meters, +x stage-left, +y upstage;
 * results are rounded to the millimeter. Z is never touched.
 */

export type Vec3 = [number, number, number];

export interface Placed {
  name: string;
  position: Vec3;
  /** Mounting rotation in degrees, when the fixture has one. Only mirroring
   *  changes it. */
  rotation?: Vec3;
}

const mm = (v: number): number => {
  const r = Math.round(v * 1000) / 1000;
  return r === 0 ? 0 : r;
};

/** The axis the selection is strung along: y when it spreads further upstage
 *  than across (a side column), else x (a row). */
export function dominantAxis(items: Placed[]): "x" | "y" {
  const spread = (i: 0 | 1) => {
    const v = items.map((p) => p.position[i]);
    return Math.max(...v) - Math.min(...v);
  };
  return items.length > 0 && spread(1) > spread(0) ? "y" : "x";
}

/** Aligns the selection on its line: a column gets the mean x, a row the mean
 *  y. */
export function alignOnLine(items: Placed[]): Placed[] {
  if (items.length < 2) return items;
  const along = dominantAxis(items);
  // A column (strung along y) is aligned in x, a row (along x) in y.
  const across = along === "y" ? 0 : 1;
  const mean = items.reduce((s, p) => s + p.position[across], 0) / items.length;
  return items.map((p) => {
    const position: Vec3 = [...p.position];
    position[across] = mm(mean);
    return { name: p.name, position };
  });
}

/** Spaces the selection evenly along its dominant axis: the two extremes stay
 *  and the rest are spread between them in their current order. */
export function spaceEvenly(items: Placed[]): Placed[] {
  if (items.length < 3) return items;
  const axis = dominantAxis(items) === "y" ? 1 : 0;
  const order = items
    .map((p, i) => ({ i, v: p.position[axis] }))
    .sort((a, b) => a.v - b.v || a.i - b.i);
  const first = order[0].v;
  const step = (order[order.length - 1].v - first) / (order.length - 1);
  const out = items.map((p) => ({
    name: p.name,
    position: [...p.position] as Vec3,
  }));
  order.forEach(({ i }, rank) => {
    out[i].position[axis] = mm(first + step * rank);
  });
  return out;
}

/** A rotation reflected across x = 0: with M = diag(-1, 1, 1), the mounting
 *  Rz·Ry·Rx becomes M·Rz·Ry·Rx·M = Rz(-z)·Ry(-y)·Rx(x), so `(rx, -ry, -rz)`. A
 *  perimeter's left side (110, 0, -90) becomes (110, 0, 90). */
export function mirrorRotation(rotation: Vec3): Vec3 {
  const flip = (v: number) => (v === 0 ? 0 : -v);
  return [rotation[0], flip(rotation[1]), flip(rotation[2])];
}

/** Mirrors each fixture about the centre line x = 0, and its aim with it: a
 *  fixture that had a rotation gets the reflected one, one without stays
 *  without. */
export function mirrorAcrossCentre(items: Placed[]): Placed[] {
  return items.map((p) => ({
    name: p.name,
    position: [mm(-p.position[0]), p.position[1], p.position[2]],
    ...(p.rotation ? { rotation: mirrorRotation(p.rotation) } : {}),
  }));
}

/** Moves the selection by (dx, dy) meters. */
export function nudge(items: Placed[], dx: number, dy: number): Placed[] {
  return items.map((p) => ({
    name: p.name,
    position: [mm(p.position[0] + dx), mm(p.position[1] + dy), p.position[2]],
  }));
}
