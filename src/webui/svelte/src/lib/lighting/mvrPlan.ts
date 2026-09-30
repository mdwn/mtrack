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
 * The import wizard's plan: a top-down view of an MVR's fixtures and scenery
 * in the file's own millimeters, and the arithmetic to suggest a stage origin
 * and to turn a click into millimeters. Pure, so it is easy to test.
 * MVR space is right-handed Z-up; on the plan +x runs right and +y runs up
 * the page.
 */

import type { MvrBoundsMm, MvrSceneView, Vec3Mm } from "../api/config";

export interface PlanBox {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
}

/** The suggested origin and what it was taken from. */
export interface OriginSuggestion {
  origin: Vec3Mm;
  /** `deck`: the front edge of the scenery's stage floor. `footprint`: the
   *  front edge of the fixtures' footprint. */
  basis: "deck" | "footprint";
}

/**
 * The origin to offer: the middle of the deck's front edge (its lowest y) at
 * the deck's top when the scenery carries a deck, else the middle of the
 * front edge of the fixtures' footprint at the floor. Null when the file has
 * nothing positioned to go by.
 */
export function suggestOrigin(scene: MvrSceneView): OriginSuggestion | null {
  const deck = scene.deck_mm;
  if (deck) {
    return {
      origin: [
        Math.round((deck.min[0] + deck.max[0]) / 2),
        Math.round(deck.min[1]),
        Math.round(deck.max[2]),
      ],
      basis: "deck",
    };
  }
  const points = scene.fixtures
    .map((f) => f.position_mm)
    .filter((p): p is Vec3Mm => p !== null);
  if (points.length === 0) return null;
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  return {
    origin: [
      Math.round((Math.min(...xs) + Math.max(...xs)) / 2),
      Math.round(Math.min(...ys)),
      0,
    ],
    basis: "footprint",
  };
}

/** Everything the plan draws, padded, so nothing sits on the edge. Null when
 *  there is nothing to draw. */
export function planBox(scene: MvrSceneView): PlanBox | null {
  const xs: number[] = [];
  const ys: number[] = [];
  const add = (p: Vec3Mm | null) => {
    if (p) {
      xs.push(p[0]);
      ys.push(p[1]);
    }
  };
  const addBounds = (b: MvrBoundsMm | null) => {
    if (b) {
      add(b.min);
      add(b.max);
    }
  };
  for (const f of scene.fixtures) add(f.position_mm);
  for (const f of scene.focus_points) add(f.position_mm);
  for (const s of scene.scenery) addBounds(s.bounds_mm);
  addBounds(scene.deck_mm);
  if (xs.length === 0) return null;
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  // At least a metre each way, and a tenth of the span again as margin.
  const padX = Math.max(1000, (maxX - minX) * 0.1);
  const padY = Math.max(1000, (maxY - minY) * 0.1);
  return {
    minX: minX - padX,
    maxX: maxX + padX,
    minY: minY - padY,
    maxY: maxY + padY,
  };
}

/** A plan-space y to the page's (y grows down the page, up the stage). */
export function pageY(box: PlanBox, y: number): number {
  return box.maxY - y + box.minY;
}

/** A point on the page (in the plan's own units) back to millimeters. */
export function fromPage(box: PlanBox, x: number, y: number): [number, number] {
  return [Math.round(x), Math.round(box.maxY - y + box.minY)];
}

/** The default venue name for a file: its stem. */
export function defaultVenueName(fileName: string): string {
  return fileName.replace(/\.mvr$/i, "");
}

/** A name reduced to a file-name-safe stem, as the server does for
 *  `<venue>.mvr`: lowercase letters and digits, single underscores. */
export function fileStem(name: string): string {
  const stem = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return stem || "fixture";
}

/** Whether an export file name is one the server will take: a bare `.mvr`
 *  name with no directories and nothing hidden. */
export function validExportName(name: string): boolean {
  const n = name.trim();
  return (
    /\.mvr$/i.test(n) &&
    n.length > 4 &&
    !/[\\/\0]/.test(n) &&
    !n.startsWith(".")
  );
}
