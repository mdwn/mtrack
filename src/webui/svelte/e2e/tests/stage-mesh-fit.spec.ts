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

// A model mesh is drawn at its Model's declared size (GDTF: "The mesh is
// explicitly scaled to this dimension"), checked as plain numbers.

import { test, expect } from "@playwright/test";
import { Box3, BoxGeometry, Group, Mesh, Vector3 } from "three";
import { declaredScale, fittedMesh } from "../../src/lib/stage/fit";

/** A glTF-style scene (Y-up) holding one box of the given extents. */
function gltfBox(x: number, yUp: number, z: number, nodeScale = 1): Group {
  const scene = new Group();
  const node = new Group();
  node.scale.setScalar(nodeScale);
  node.add(new Mesh(new BoxGeometry(x, yUp, z)));
  scene.add(node);
  return scene;
}

function drawnSize(holder: Group): number[] {
  holder.updateMatrixWorld(true);
  const s = new Box3().setFromObject(holder).getSize(new Vector3());
  return [s.x, s.y, s.z].map((v) => Math.round(v * 1e6) / 1e6);
}

test("a mesh twice too big along one axis ends at the declared size", () => {
  // Length (x) is drawn 0.2 m against a declared 0.1 m.
  const holder = fittedMesh(gltfBox(0.2, 0.093, 0.089), [0.1, 0.089, 0.093]);
  expect(drawnSize(holder)).toEqual([0.1, 0.089, 0.093]);
});

test("Width is the mesh's glTF Z extent and Height its Y (up) extent", () => {
  // Already the declared size once turned Z-up: nothing changes.
  const holder = fittedMesh(gltfBox(0.1, 0.3, 0.2), [0.1, 0.2, 0.3]);
  for (const k of holder.scale.toArray()) expect(k).toBeCloseTo(1, 6);
  expect(drawnSize(holder)).toEqual([0.1, 0.2, 0.3]);
});

test("the mesh is measured with its own node transforms applied", () => {
  // A node scale of 10 on a 1 cm box: the mesh is 0.1 m, which is right.
  const holder = fittedMesh(gltfBox(0.01, 0.01, 0.01, 10), [0.1, 0.1, 0.1]);
  expect(drawnSize(holder)).toEqual([0.1, 0.1, 0.1]);
  // And a mesh authored in millimetres is brought to meters.
  const mm = fittedMesh(gltfBox(89, 93, 89), [0.089, 0.089, 0.093]);
  expect(drawnSize(mm)).toEqual([0.089, 0.089, 0.093]);
});

test("a Model that states no size leaves the mesh as its file has it", () => {
  for (const size of [[0, 0, 0], [0.1, 0, 0.1], null, undefined] as (
    | [number, number, number]
    | null
    | undefined
  )[]) {
    const holder = fittedMesh(gltfBox(0.4, 0.2, 0.3), size);
    expect(holder.scale.toArray()).toEqual([1, 1, 1]);
    expect(drawnSize(holder)).toEqual([0.4, 0.3, 0.2]);
  }
});

test("a flat axis is left alone rather than blown up", () => {
  expect(declaredScale([0.2, 0.1, 0], [0.1, 0.1, 0.002])).toEqual([0.5, 1, 1]);
});
