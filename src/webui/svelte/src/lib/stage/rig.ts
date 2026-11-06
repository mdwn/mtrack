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

import type { Matrix4, RigModel } from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

/**
 * The rig model as the asset store writes it (design §16.2), and the
 * math that turns a rig plus a pose into rotations — kept free of three.js
 * so it can be reasoned about (and tested) as plain numbers.
 *
 * Conventions, from the GDTF spec (design §18): a rig's coordinates are
 * Z-up with the fixture drawn hanging, base up; a beam leaves its node
 * along −Z; pan turns the pan node about its local Z; tilt turns the tilt
 * node about its local X. Pose degrees are GDTF's own, so nothing here
 * remaps them:
 *
 * - Rest (pan 0, tilt 0) is the mounting frame's −Z: straight down for a
 *   hung fixture. This holds for static fixtures too — an unrotated PAR
 *   points down, as its GDTF says.
 * - Positive pan is a right-hand rotation about +Z: counter-clockwise
 *   seen from above.
 * - Positive tilt is a right-hand rotation about +X: it swings the beam
 *   from −Z toward +Y.
 *
 * A venue's `rotation` is the mounting exactly as GDTF and MVR model it,
 * applied outside the joints as R = Rz·Ry·Rx, so a fixture's beam is
 * `R · Rz(pan) · Rx(tilt) · (0, 0, −1)`.
 */

/** The beam angle a rig without photometrics is drawn with, degrees. */
export const GENERIC_BEAM_DEG = 20;

/**
 * A stand-in for a fixture type with no rig: a box body with one beam
 * along the mounting frame's −Z, the rest direction every GDTF fixture
 * has — straight down for a hung fixture.
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
        // Unrotated, below the body: local −Z is the parent's −Z.
        transform: [
          [1, 0, 0, 0],
          [0, 1, 0, 0],
          [0, 0, 1, -0.16],
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

export function identity(): Matrix4 {
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
 * about the pan node's local Z, tilt about the tilt node's local X.
 *
 * Pose degrees are GDTF's, so this is the identity in radians. Rest
 * (pan 0, tilt 0) leaves the beam on the mounting frame's −Z — straight
 * down for a hung fixture. Positive pan is a right-hand rotation about
 * +Z, counter-clockwise seen from above. Positive tilt is a right-hand
 * rotation about +X, swinging the beam from −Z toward +Y. A rig without
 * axes ignores the pose.
 */
export function poseRotations(
  rig: RigModel,
  pan: number,
  tilt: number,
): { panZ: number; tiltX: number } {
  if (!isMover(rig)) return { panZ: 0, tiltX: 0 };
  return {
    panZ: (pan * Math.PI) / 180,
    tiltX: (tilt * Math.PI) / 180,
  };
}

/** How long a spot's beam is drawn when it points up or level, meters. */
export const SKY_BEAM_LENGTH = 4;

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

/** What the stage views draw a fixture as; see {@link beamLook}. */
export interface BeamLook {
  /** The colour at its level, each 0..1. */
  rgb: [number, number, number];
  intensity: number;
  /** False in the dark half of a strobe's cycle. */
  strobeOn: boolean;
  /** Whether it is lit: the engine's test of a lit fixture
   *  (`rules.fixture_lit`), outside a strobe's dark half. */
  lit: boolean;
}

/**
 * The colour and level a fixture's state shows, the one reading every
 * stage view (2D plot, timeline preview, 3D scene) draws from: the
 * player's own (`rules.fixture_color`, which the TUI draws by too) --
 * each part that lights the fixture in its colour times its dimmer, white
 * added, a dimmer alone a warm white -- and lit as the player has it
 * (`rules.fixture_lit`). The level is the colour's mean.
 *
 * `strobeHz` is the rate the player says the fixture is flashing at (the
 * state's `strobe_rates`: its strobe byte read back through its own strobe
 * rule, or in an evaluation a software strobe's rate), and the look is
 * dark for half of each cycle at `now` (ms). The view does not guess a
 * rate from the byte: the fixture's range and curve are the player's to
 * know.
 *
 * `undefined` channels (nothing reported for the fixture) is dark, not
 * full.
 */
export function beamLook(
  channels: Record<string, number | undefined> | undefined,
  strobeHz?: number,
  now: number = Date.now(),
): BeamLook {
  // No state at all for the fixture (idle player, nothing reported yet):
  // dark.
  if (!channels)
    return { rgb: [0, 0, 0], intensity: 0, strobeOn: true, lit: false };
  const rgb = rules.fixture_color(channels);
  let strobeOn = true;
  const hz = strobeHz ?? 0;
  if (hz > 0) {
    strobeOn = Math.sin((now / 1000) * hz * Math.PI * 2) > 0;
  }
  return {
    rgb,
    intensity: (rgb[0] + rgb[1] + rgb[2]) / 3,
    strobeOn,
    lit: strobeOn && rules.fixture_lit(channels),
  };
}

/** A look as a canvas colour: its colour, or black in a strobe's dark half. */
export function beamCss(look: BeamLook): string {
  if (!look.strobeOn) return "rgb(0,0,0)";
  const [r, g, b] = look.rgb.map((c) => Math.round(c * 255));
  return `rgb(${r},${g},${b})`;
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
