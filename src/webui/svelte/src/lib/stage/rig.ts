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
 * The rig model as the asset store writes it (design §16.2), and the
 * math that turns a rig plus a pose into rotations — kept free of three.js
 * so it can be reasoned about (and tested) as plain numbers.
 *
 * Conventions, from the GDTF spec and the P1c pointing convention: a rig's
 * coordinates are Z-up with the fixture hanging base-up, the head pointing
 * −Z at rest; a beam leaves its node along −Z; pan turns the pan node
 * about its local Z; tilt turns the tilt node about its local X. The venue
 * says pan 0, tilt 0 looks along the mounting frame's +y, level — so the
 * tilt node is raised 90° from rest, and pan runs the other way round Z.
 */

export type Mat4 = [
  [number, number, number, number],
  [number, number, number, number],
  [number, number, number, number],
  [number, number, number, number],
];

export type RigRole =
  | { kind: "body" }
  | { kind: "pan" }
  | { kind: "tilt" }
  | { kind: "beam" }
  | { kind: "cell"; index: number };

export type RigShape =
  | { shape: "model"; file: string }
  | { shape: "primitive"; kind: string; size: [number, number, number] }
  | { shape: "empty" };

export interface RigNode {
  name: string;
  parent: number | null;
  role: RigRole;
  /** Row-major 4×4, translation in the fourth column, meters. */
  transform: Mat4;
  shape: RigShape;
}

export interface RigBeam {
  node: number;
  angle_deg: number;
  field_deg: number | null;
  kind: string;
  flux_lm: number | null;
  cct_k: number | null;
  radius_m: number | null;
}

export interface RigModel {
  version: number;
  fixture_type: string;
  mode: string;
  nodes: RigNode[];
  pan: number | null;
  tilt: number | null;
  beams: RigBeam[];
  thumbnail: string | null;
  warnings?: string[];
}

/** A venue's scenery as the asset store writes it (design §16.3). */
export interface SceneryModel {
  version: number;
  objects: SceneryObject[];
  formats: Record<string, number>;
  warnings?: string[];
}

export interface SceneryObject {
  name: string;
  kind: string;
  layer: string;
  /** Row-major 4×4 in stage space, translation in meters. */
  transform: Mat4;
  meshes: { file: string; transform: Mat4 }[];
  /** Mesh files the store could not hold (formats it does not draw). */
  skipped: string[];
}

/** The beam angle a rig without photometrics is drawn with, degrees. */
export const GENERIC_BEAM_DEG = 20;

/**
 * A stand-in for a fixture type with no rig: a box body with one beam
 * along the mounting frame's +y (the direction the stage plot's tick
 * draws), which is −Z of a node raised 90° about X.
 */
export function genericRig(typeName: string): RigModel {
  return {
    version: 0,
    fixture_type: typeName,
    mode: "",
    nodes: [
      {
        name: "Body",
        parent: null,
        role: { kind: "body" },
        transform: identity(),
        shape: { shape: "primitive", kind: "Cube", size: [0.3, 0.3, 0.3] },
      },
      {
        name: "Lens",
        parent: 0,
        role: { kind: "beam" },
        // Raised 90° about X: local −Z becomes parent +y.
        transform: [
          [1, 0, 0, 0],
          [0, 0, -1, 0.16],
          [0, 1, 0, 0],
          [0, 0, 0, 1],
        ],
        shape: { shape: "empty" },
      },
    ],
    pan: null,
    tilt: null,
    beams: [
      {
        node: 1,
        angle_deg: GENERIC_BEAM_DEG,
        field_deg: null,
        kind: "Wash",
        flux_lm: null,
        cct_k: null,
        radius_m: null,
      },
    ],
    thumbnail: null,
  };
}

export function identity(): Mat4 {
  return [
    [1, 0, 0, 0],
    [0, 1, 0, 0],
    [0, 0, 1, 0],
    [0, 0, 0, 1],
  ];
}

/** Whether a rig moves: it has a tilt node (a pan alone is a scanner's). */
export function isMover(rig: RigModel): boolean {
  return rig.tilt !== null;
}

/**
 * The rotations, in radians, the pan and tilt nodes get for a pose: pan
 * about the pan node's local Z, tilt about the tilt node's local X. A rig
 * without axes ignores the pose.
 */
export function poseRotations(
  rig: RigModel,
  pan: number,
  tilt: number,
): { panZ: number; tiltX: number } {
  if (!isMover(rig)) return { panZ: 0, tiltX: 0 };
  return {
    panZ: (-pan * Math.PI) / 180,
    tiltX: ((tilt + 90) * Math.PI) / 180,
  };
}

/**
 * How the rig's root sits in the mounting frame. A mover hangs as the GDTF
 * models it (pan axis vertical). A static rig — a PAR, a blinder — is
 * raised so its rest beam (−Z) runs along the mounting frame's +y, the
 * direction the venue convention and the stage plot agree on.
 */
export function rootTiltX(rig: RigModel): number {
  return isMover(rig) ? 0 : Math.PI / 2;
}

/** Where a beam meets the deck (z = 0) from a point along a direction. */
export function beamLength(
  origin: [number, number, number],
  direction: [number, number, number],
  options: { maxLength: number; skyLength: number },
): { length: number; onDeck: boolean } {
  if (direction[2] < -1e-4) {
    const toDeck = origin[2] / -direction[2];
    if (toDeck <= options.maxLength) return { length: toDeck, onDeck: true };
    return { length: options.maxLength, onDeck: false };
  }
  return { length: options.skyLength, onDeck: false };
}

/**
 * The colour and level a fixture's channels show, as the 2D view reads
 * them: `red`/`green`/`blue` (+ `white`) mixed by `dimmer`, or a warm
 * white by `dimmer` alone. Other colour systems (CMY, amber, colour
 * wheels) are not read — their fixtures show as a dimmer-only white.
 */
export function beamLook(channels: {
  red?: number;
  green?: number;
  blue?: number;
  dimmer?: number;
  strobe?: number;
  white?: number;
}): { rgb: [number, number, number]; intensity: number; strobeOn: boolean } {
  const dimmer = (channels.dimmer ?? 255) / 255;
  const hasColor =
    channels.red !== undefined ||
    channels.green !== undefined ||
    channels.blue !== undefined;
  // A dimmer-only fixture is a warm white; a colour fixture shows its mix.
  let r = hasColor ? (channels.red ?? 0) / 255 : 1;
  let g = hasColor ? (channels.green ?? 0) / 255 : 0.92;
  let b = hasColor ? (channels.blue ?? 0) / 255 : 0.8;
  if (channels.white !== undefined) {
    const w = channels.white / 255;
    r = Math.min(1, r + w);
    g = Math.min(1, g + w);
    b = Math.min(1, b + w);
  }
  let strobeOn = true;
  const strobe = channels.strobe ?? 0;
  if (strobe > 10) {
    const freq = 2 + (strobe / 255) * 18;
    strobeOn = Math.sin((Date.now() / 1000) * freq * Math.PI * 2) > 0;
  }
  const intensity = ((r + g + b) / 3) * dimmer;
  return { rgb: [r * dimmer, g * dimmer, b * dimmer], intensity, strobeOn };
}

/**
 * Where fixtures the venue does not place are shown: a tray downstage of
 * the deck edge, in name order, so they are seen and obviously unplaced.
 */
export function trayPositions(
  names: string[],
  spacing = 0.8,
): Record<string, [number, number, number]> {
  const sorted = [...names].sort();
  const width = (sorted.length - 1) * spacing;
  const out: Record<string, [number, number, number]> = {};
  sorted.forEach((name, i) => {
    out[name] = [-width / 2 + i * spacing, -1.5, 0.6];
  });
  return out;
}

/**
 * The deck extent that holds everything, meters: [minX, maxX, minY, maxY].
 * The deck starts at the audience edge (y = 0) unless something sits
 * downstage of it — an MVR whose origin was left mid-stage — in which
 * case it reaches down to hold that too.
 */
export function deckExtent(
  points: [number, number, number][],
  minimum: [number, number] = [8, 6],
): [number, number, number, number] {
  let maxAbsX = minimum[0] / 2;
  let maxY = minimum[1];
  let minY = 0;
  for (const [x, y] of points) {
    maxAbsX = Math.max(maxAbsX, Math.abs(x) + 1);
    maxY = Math.max(maxY, y + 1);
    minY = Math.min(minY, y - 1);
  }
  return [-maxAbsX, maxAbsX, minY, maxY];
}
