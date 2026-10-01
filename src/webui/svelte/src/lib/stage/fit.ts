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
 * A GDTF model's mesh at the size its Model declares. The spec: "The
 * dimension XML attributes of model are always used, no matter the scaling
 * and ratio of the mesh file. The mesh is explicitly scaled to this
 * dimension" — Length on X, Width on Y, Height on Z. Blender DMX (the
 * repo's external check) does exactly that, per axis, measuring the mesh
 * with its node transforms applied after turning glTF's Y-up to Z-up.
 * A declared size of 0 is the spec's default, "not stated": the mesh is
 * then drawn at the size its file has.
 */

import * as THREE from "three";

export type Size3 = [number, number, number];

/** The per-axis scale that takes a mesh's extent to the declared size, or
 *  null when the Model states no size (any dimension 0 or missing). An
 *  axis the mesh has no extent on (a flat plate) is left at 1. */
export function declaredScale(
  extent: Size3,
  size: Size3 | null | undefined,
): Size3 | null {
  if (!size || !size.every((s) => Number.isFinite(s) && s > 0)) return null;
  return extent.map((e, i) => (e > 1e-9 ? size[i] / e : 1)) as Size3;
}

/**
 * A glTF scene turned Z-up and, when the Model declares a size, scaled per
 * axis to it about the mesh's own origin. Returns the holder to add to the
 * rig node; the mesh's own frame (its node transforms) is measured before
 * the holder is attached anywhere, so the box is the mesh's alone.
 */
export function fittedMesh(
  mesh: THREE.Object3D,
  size: Size3 | null | undefined,
): THREE.Group {
  // glTF is Y-up; the rig is Z-up.
  mesh.rotation.x = Math.PI / 2;
  const holder = new THREE.Group();
  holder.add(mesh);
  holder.updateMatrixWorld(true);
  const box = new THREE.Box3().setFromObject(mesh);
  if (box.isEmpty()) return holder;
  const extent = box.getSize(new THREE.Vector3());
  const scale = declaredScale([extent.x, extent.y, extent.z], size);
  if (scale) holder.scale.set(scale[0], scale[1], scale[2]);
  return holder;
}
