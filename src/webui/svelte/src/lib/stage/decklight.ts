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
 * The light the beams put on the deck, as plain numbers (no three.js): the
 * 3D view's cones show a beam in the air, and this is what the same beam
 * does to the floor.
 *
 * A beam is not asked where its centre lands. Every point of the deck is
 * lit by the whole cone: full inside the beam angle, fading to nothing at
 * the field angle, with a little spill beyond; weaker with distance and
 * with how obliquely it strikes. So a fixture on the floor aimed up at the
 * band, whose centre never meets the deck, still lights the floor in front
 * of it with the lower edge of its beam — the wider the beam, the more.
 *
 * Brightness is relative, not photometric: every fixture is taken to put
 * out the same light, so a narrow beam is brighter than a wide one, and
 * the result is compressed the way an eye adapts. The formula is written
 * twice, here ({@link deckLight}) and in {@link DECK_FRAGMENT} for the
 * GPU, from the same constants.
 */

type Vec3 = [number, number, number];

/** The field angle taken for a beam whose type states none, as a multiple
 *  of its beam angle; also the most a stated ratio is trusted for. */
export const DEFAULT_FIELD_RATIO = 1.6;
const MAX_FIELD_RATIO = 2;
/** The widest cone drawn in the air, degrees: past this a cone is a wall. */
export const MAX_CONE_DEG = 120;
/** Light outside the field (the lens's scatter, the housing's glow), as a
 *  fraction of the reference beam's centre: the same for any beam, since
 *  a narrow lens does not scatter more. */
export const SPILL = 0.02;
/** Nearer than this, meters, a source is no brighter: it has a size. */
export const NEAR = 0.3;
/** The narrowest beam taken when sharing a fixture's light over its cone,
 *  degrees, so a pencil beam does not outshine everything else. */
const MIN_STRENGTH_DEG = 8;
/** The beam the brightness is normalised to: a 25° fixture straight down
 *  from {@link REFERENCE_HEIGHT} meters at full lights the deck to 1. */
const REFERENCE_BEAM_DEG = 25;
const REFERENCE_HEIGHT = 4;
/** The light level drawn at half brightness (the eye's adaptation), in
 *  the reference's units, and the brightest the deck is drawn. */
export const HALF_LEVEL = 0.3;
export const GAIN = 0.85;
/** The most beams the deck is lit by at once; the strongest are kept. */
export const MAX_DECK_LIGHTS = 64;
/** Floats per light in the packed buffer: three RGBA texels. */
export const LIGHT_FLOATS = 12;

/** The beam and field angles a beam is drawn with, degrees: the rig's,
 *  or the venue fixture's own beam angle (a diffuser or filter fitted to
 *  that unit) with the field keeping the rig's proportion. */
export function beamSpread(
  rig: { angle_deg: number; field_deg: number | null },
  override?: number | null,
): { beamDeg: number; fieldDeg: number } {
  const stated =
    rig.field_deg !== null && rig.angle_deg > 0 && rig.field_deg > rig.angle_deg
      ? Math.min(rig.field_deg / rig.angle_deg, MAX_FIELD_RATIO)
      : DEFAULT_FIELD_RATIO;
  const valid = typeof override === "number" && override > 0 && override <= 180;
  const beamDeg = Math.min(valid ? override : rig.angle_deg, 180);
  return { beamDeg, fieldDeg: Math.min(beamDeg * stated, 180) };
}

function solidAngle(beamDeg: number, fieldDeg: number): number {
  // The cone that holds the light: halfway between beam and field.
  const half = (Math.max(beamDeg, MIN_STRENGTH_DEG) + fieldDeg) / 4;
  return 2 * Math.PI * (1 - Math.cos((half * Math.PI) / 180));
}

/** How bright a beam's centre is, relative to the reference beam's at a
 *  meter: the same light through a wider cone is thinner. */
export function beamStrength(beamDeg: number, fieldDeg: number): number {
  const reference = solidAngle(
    REFERENCE_BEAM_DEG,
    REFERENCE_BEAM_DEG * DEFAULT_FIELD_RATIO,
  );
  return (
    (REFERENCE_HEIGHT * REFERENCE_HEIGHT * reference) /
    solidAngle(beamDeg, fieldDeg)
  );
}

/** The spill's strength, in {@link beamStrength}'s units. */
const SPILL_STRENGTH = SPILL * REFERENCE_HEIGHT * REFERENCE_HEIGHT;

/** One beam, as the deck sees it. */
export interface DeckLight {
  /** The lens, stage meters. */
  origin: Vec3;
  /** Unit vector along the beam. */
  aim: Vec3;
  beamDeg: number;
  fieldDeg: number;
  /** The beam's colour at its level (0–1 per channel). */
  rgb: Vec3;
  /** This beam's part of its fixture's light (1 for a single lens). */
  share: number;
}

const cosHalf = (deg: number) => Math.cos((deg * Math.PI) / 360);

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

/** The light one beam puts on the deck at (x, y), per channel, before the
 *  eye's compression: 1 is the reference beam's centre. */
export function deckLight(light: DeckLight, x: number, y: number): Vec3 {
  const v: Vec3 = [x - light.origin[0], y - light.origin[1], -light.origin[2]];
  const length = Math.hypot(v[0], v[1], v[2]);
  if (length < 1e-6) return [0, 0, 0];
  const dir = v.map((c) => c / length) as Vec3;
  const cosA =
    dir[0] * light.aim[0] + dir[1] * light.aim[1] + dir[2] * light.aim[2];
  const cosBeam = cosHalf(light.beamDeg);
  const cosField = Math.min(cosHalf(light.fieldDeg), cosBeam - 1e-4);
  const cone = smoothstep(cosField, cosBeam, cosA);
  const spill = SPILL_STRENGTH * Math.max(cosA, 0);
  // The deck faces up: light from below it, or along it, does not land.
  const incidence = Math.max(-dir[2], 0);
  const level =
    (light.share *
      Math.max(beamStrength(light.beamDeg, light.fieldDeg) * cone, spill) *
      incidence) /
    Math.max(length * length, NEAR * NEAR);
  return [light.rgb[0] * level, light.rgb[1] * level, light.rgb[2] * level];
}

/** The colour drawn for a summed light: compressed on its brightest
 *  channel, so a strong light saturates without changing hue. */
export function deckColor(sum: Vec3): Vec3 {
  const peak = Math.max(sum[0], sum[1], sum[2]);
  if (peak <= 0) return [0, 0, 0];
  const scale = GAIN / (peak + HALF_LEVEL);
  return [sum[0] * scale, sum[1] * scale, sum[2] * scale];
}

/** How much a light matters, for keeping the strongest. */
function weight(light: DeckLight): number {
  return (
    beamStrength(light.beamDeg, light.fieldDeg) *
    light.share *
    Math.max(light.rgb[0], light.rgb[1], light.rgb[2])
  );
}

/**
 * Packs the lights into `out` for the shader, three RGBA texels each:
 * (origin, cos half beam), (aim, cos half field), (colour × share,
 * strength).
 * Lights that are dark or under the deck are left out; past
 * {@link MAX_DECK_LIGHTS} the strongest are kept. Returns how many.
 */
export function packDeckLights(
  lights: readonly DeckLight[],
  out: Float32Array,
): number {
  let kept = lights.filter(
    (l) => l.origin[2] > 0 && Math.max(l.rgb[0], l.rgb[1], l.rgb[2]) > 0,
  );
  if (kept.length > MAX_DECK_LIGHTS) {
    kept = kept
      .map((light) => ({ light, weight: weight(light) }))
      .sort((a, b) => b.weight - a.weight)
      .slice(0, MAX_DECK_LIGHTS)
      .map((entry) => entry.light);
  }
  kept.forEach((light, i) => {
    const cosBeam = cosHalf(light.beamDeg);
    out.set(
      [
        ...light.origin,
        cosBeam,
        ...light.aim,
        Math.min(cosHalf(light.fieldDeg), cosBeam - 1e-4),
        light.rgb[0] * light.share,
        light.rgb[1] * light.share,
        light.rgb[2] * light.share,
        beamStrength(light.beamDeg, light.fieldDeg),
      ],
      i * LIGHT_FLOATS,
    );
  });
  return kept.length;
}

export const DECK_VERTEX = /* glsl */ `
varying vec3 vWorld;
void main() {
  vec4 world = modelMatrix * vec4(position, 1.0);
  vWorld = world.xyz;
  gl_Position = projectionMatrix * viewMatrix * world;
}
`;

/** {@link deckLight} and {@link deckColor}, summed over the packed lights. */
export const DECK_FRAGMENT = /* glsl */ `
precision highp sampler2D;
uniform sampler2D uLights;
uniform int uCount;
varying vec3 vWorld;
void main() {
  vec3 sum = vec3(0.0);
  for (int i = 0; i < ${MAX_DECK_LIGHTS}; i++) {
    if (i >= uCount) break;
    vec4 origin = texelFetch(uLights, ivec2(0, i), 0);
    vec4 aim = texelFetch(uLights, ivec2(1, i), 0);
    vec4 color = texelFetch(uLights, ivec2(2, i), 0);
    vec3 v = vec3(vWorld.xy, 0.0) - origin.xyz;
    float len = max(length(v), 1e-6);
    vec3 dir = v / len;
    float cosA = dot(dir, aim.xyz);
    float cone = color.a * smoothstep(aim.w, origin.w, cosA);
    float spill = ${SPILL_STRENGTH.toFixed(4)} * max(cosA, 0.0);
    float incidence = max(-dir.z, 0.0);
    sum += color.rgb * max(cone, spill) * incidence
      / max(len * len, ${(NEAR * NEAR).toFixed(4)});
  }
  float peak = max(sum.r, max(sum.g, sum.b));
  gl_FragColor = vec4(
    sum * (${GAIN.toFixed(4)} / (peak + ${HALF_LEVEL.toFixed(4)})), 1.0);
  #include <colorspace_fragment>
}
`;
