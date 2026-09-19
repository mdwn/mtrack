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
 * Stage layout shared by the dashboard's stage view and the timeline
 * editor's preview.
 *
 * Two modes. When the venue places its fixtures (or names focus points),
 * the plot is a top-down stage in stage coordinates: meters, origin
 * downstage-center, +x stage-left, +y upstage. Seen from the audience,
 * stage-left is on the right of the picture and upstage is at the top, so
 * x maps straight across and y is flipped. Without geometry the layout is
 * the older tag heuristic: `left`/`right`/`front`/`back`/`mid` tags pick a
 * region, everything else lands on the front row.
 */

export type Vec3 = [number, number, number];

export interface Pt {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface StageFixtureMeta {
  tags?: string[];
  position?: Vec3 | null;
}

/** Meters-to-pixels mapping for a plot rectangle. */
export interface StageFrame {
  /** Pixels per meter. */
  scale: number;
  /** Pixel position of stage (0, 0). */
  ox: number;
  oy: number;
  /** Stage extent shown, meters. */
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
}

/** Whether a venue has anything to plot in stage coordinates. */
export function hasGeometry(
  fixtures: Record<string, StageFixtureMeta>,
  focusPoints: Record<string, Vec3>,
): boolean {
  if (Object.keys(focusPoints).length > 0) return true;
  return Object.values(fixtures).some((f) => f.position != null);
}

/** The older tag-region layout, positions for every fixture. */
export function tagLayout(
  fixtures: Record<string, StageFixtureMeta>,
  w: number,
  h: number,
  inset: number,
): Record<string, Pt> {
  const names = Object.keys(fixtures);
  const out: Record<string, Pt> = {};
  if (names.length === 0) return out;

  const groups: Record<string, string[]> = {
    left: [],
    right: [],
    front: [],
    back: [],
    mid: [],
    other: [],
  };
  for (const name of names) {
    const tags = fixtures[name].tags || [];
    let placed = false;
    for (const tag of tags) {
      const key = tag.toLowerCase();
      if (key in groups && key !== "other") {
        groups[key].push(name);
        placed = true;
        break;
      }
    }
    if (!placed) groups.other.push(name);
  }
  groups.front = groups.front.concat(groups.other);

  const regions: Record<
    string,
    { x: number; y: number; dx: number; dy: number }
  > = {
    left: { x: inset, y: h * 0.25, dx: 0, dy: h * 0.5 },
    right: { x: w - inset, y: h * 0.25, dx: 0, dy: h * 0.5 },
    back: { x: w * 0.25, y: inset, dx: w * 0.5, dy: 0 },
    front: { x: w * 0.25, y: h - inset, dx: w * 0.5, dy: 0 },
    mid: { x: w * 0.35, y: h * 0.4, dx: w * 0.3, dy: h * 0.2 },
  };
  for (const [groupName, region] of Object.entries(regions)) {
    const group = groups[groupName];
    if (!group || group.length === 0) continue;
    const count = group.length;
    for (let i = 0; i < count; i++) {
      const t = count === 1 ? 0.5 : i / (count - 1);
      out[group[i]] = {
        x: region.x + region.dx * t,
        y: region.y + region.dy * t,
      };
    }
  }
  return out;
}

/** The smallest stage shown, meters, so a two-fixture venue is not a blob. */
const MIN_WIDTH_M = 6;
const MIN_DEPTH_M = 4;
/** Room around the outermost fixture, meters. */
const MARGIN_M = 1;

/**
 * Fits the venue's geometry into a plot rectangle with a uniform scale,
 * centered. The origin (downstage-center) is always inside the shown
 * extent so the audience edge reads as the bottom of the picture.
 */
export function fitFrame(
  fixtures: Record<string, StageFixtureMeta>,
  focusPoints: Record<string, Vec3>,
  plot: Rect,
): StageFrame {
  const xs: number[] = [0];
  const ys: number[] = [0];
  for (const f of Object.values(fixtures)) {
    if (f.position) {
      xs.push(f.position[0]);
      ys.push(f.position[1]);
    }
  }
  for (const p of Object.values(focusPoints)) {
    xs.push(p[0]);
    ys.push(p[1]);
  }
  let minX = Math.min(...xs) - MARGIN_M;
  let maxX = Math.max(...xs) + MARGIN_M;
  const minY = Math.min(...ys) - MARGIN_M;
  let maxY = Math.max(...ys) + MARGIN_M;
  // Symmetric about the centerline, so the picture is a stage and not a
  // crop of wherever the fixtures happen to be.
  const halfWidth = Math.max(Math.abs(minX), Math.abs(maxX), MIN_WIDTH_M / 2);
  minX = -halfWidth;
  maxX = halfWidth;
  if (maxY - minY < MIN_DEPTH_M) maxY = minY + MIN_DEPTH_M;

  const scale = Math.min(plot.w / (maxX - minX), plot.h / (maxY - minY));
  const shownW = (maxX - minX) * scale;
  const shownH = (maxY - minY) * scale;
  const left = plot.x + (plot.w - shownW) / 2;
  const top = plot.y + (plot.h - shownH) / 2;
  return {
    scale,
    ox: left - minX * scale,
    oy: top + maxY * scale,
    minX,
    maxX,
    minY,
    maxY,
  };
}

/** Stage meters to canvas pixels (y flipped: upstage is up). */
export function toPx(frame: StageFrame, v: Vec3 | [number, number]): Pt {
  return { x: frame.ox + v[0] * frame.scale, y: frame.oy - v[1] * frame.scale };
}

/** Canvas pixels to stage meters, to the centimeter. */
export function toStage(frame: StageFrame, pt: Pt): [number, number] {
  const x = (pt.x - frame.ox) / frame.scale;
  const y = (frame.oy - pt.y) / frame.scale;
  return [Math.round(x * 100) / 100, Math.round(y * 100) / 100];
}

/** Positions for the placed fixtures, and the names of the unplaced. */
export function positionalLayout(
  fixtures: Record<string, StageFixtureMeta>,
  frame: StageFrame,
): { placed: Record<string, Pt>; unplaced: string[] } {
  const placed: Record<string, Pt> = {};
  const unplaced: string[] = [];
  for (const [name, f] of Object.entries(fixtures)) {
    if (f.position) placed[name] = toPx(frame, f.position);
    else unplaced.push(name);
  }
  unplaced.sort();
  return { placed, unplaced };
}

/** A row of slots for the unplaced fixtures, evenly spaced across a tray. */
export function trayLayout(
  names: string[],
  tray: Rect,
  radius: number,
): Record<string, Pt> {
  const out: Record<string, Pt> = {};
  if (names.length === 0) return out;
  const step = Math.min(radius * 3.2, tray.w / names.length);
  const start = tray.x + tray.w / 2 - (step * (names.length - 1)) / 2;
  names.forEach((name, i) => {
    out[name] = { x: start + i * step, y: tray.y + tray.h / 2 };
  });
  return out;
}

/**
 * Where a fixture faces on the plot, from its yaw: rotation about Z, with
 * an unrotated fixture facing upstage, so the seeded `rotation (0, 0, 180)`
 * of a rear fixture faces the audience. Unit vector in canvas pixels.
 */
export function facing(rotation: Vec3 | null | undefined): Pt {
  const yaw = ((rotation?.[2] ?? 0) * Math.PI) / 180;
  // Stage: (-sin, cos) rotated counterclockwise from +y; canvas flips y.
  return { x: -Math.sin(yaw), y: -Math.cos(yaw) };
}

/**
 * The on-plot beam of a mover: from the fixture to its footprint on the
 * deck when the beam points down, or a short arrow along its heading when
 * it does not. Stage meters in, stage meters out.
 */
export function beamEnd(
  position: Vec3,
  aim: Vec3,
  floor: [number, number] | null,
): [number, number] {
  if (floor) return floor;
  const len = Math.hypot(aim[0], aim[1]);
  if (len < 1e-6) return [position[0], position[1]];
  const reach = 1.5;
  return [
    position[0] + (aim[0] / len) * reach,
    position[1] + (aim[1] / len) * reach,
  ];
}

/** The channels a beam's look is read from. */
export interface BeamChannels {
  red?: number;
  green?: number;
  blue?: number;
  dimmer?: number;
}

/**
 * Draws a mover's beam on a canvas: from the fixture to `end` in the color
 * the fixture is showing, solid with a footprint dot when the beam meets
 * the deck, dashed when it points up or level, neutral when the fixture
 * is dark.
 */
export function drawBeam(
  ctx: CanvasRenderingContext2D,
  from: Pt,
  end: Pt,
  channels: BeamChannels,
  hasFloor: boolean,
  options: { dark: boolean; width: number; dot: number },
): void {
  const k = (channels.dimmer ?? 255) / 255;
  const r = Math.round((channels.red ?? 0) * k);
  const g = Math.round((channels.green ?? 0) * k);
  const b = Math.round((channels.blue ?? 0) * k);
  const lit = r + g + b > 24;
  const neutral = options.dark ? "255,255,255" : "0,0,0";
  ctx.strokeStyle = lit ? `rgba(${r},${g},${b},0.55)` : `rgba(${neutral},0.18)`;
  ctx.lineWidth = lit ? options.width : Math.max(1, options.width / 2);
  ctx.setLineDash(hasFloor ? [] : [3, 3]);
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(end.x, end.y);
  ctx.stroke();
  ctx.setLineDash([]);
  if (hasFloor && options.dot > 0) {
    ctx.fillStyle = lit ? `rgba(${r},${g},${b},0.35)` : `rgba(${neutral},0.12)`;
    ctx.beginPath();
    ctx.arc(end.x, end.y, options.dot, 0, Math.PI * 2);
    ctx.fill();
  }
}

/**
 * Whether a fixture's cells lie along a line, and how to draw them if so:
 * the cells ordered along that line and the line's angle on the plot
 * (canvas radians, the fixture's yaw applied, seen from the audience).
 * Cells that spread in two directions — a ring, a grid — are not a bar.
 */
export function cellBar(
  cells: { name: string; offset: Vec3 }[],
  rotation: Vec3 | null | undefined,
): { ordered: { name: string; offset: Vec3 }[]; angle: number } | null {
  if (cells.length < 2) return null;
  const xs = cells.map((c) => c.offset[0]);
  const ys = cells.map((c) => c.offset[1]);
  const mean = (v: number[]) => v.reduce((a, b) => a + b, 0) / v.length;
  const mx = mean(xs);
  const my = mean(ys);
  // Principal direction of the offsets in the fixture's plane.
  let sxx = 0;
  let syy = 0;
  let sxy = 0;
  for (let i = 0; i < cells.length; i++) {
    const dx = xs[i] - mx;
    const dy = ys[i] - my;
    sxx += dx * dx;
    syy += dy * dy;
    sxy += dx * dy;
  }
  const total = sxx + syy;
  if (total < 1e-9) return null;
  const angle = 0.5 * Math.atan2(2 * sxy, sxx - syy);
  const ux = Math.cos(angle);
  const uy = Math.sin(angle);
  let along = 0;
  let across = 0;
  for (let i = 0; i < cells.length; i++) {
    const dx = xs[i] - mx;
    const dy = ys[i] - my;
    const a = dx * ux + dy * uy;
    const b = -dx * uy + dy * ux;
    along += a * a;
    across += b * b;
  }
  if (across > along * 0.05) return null;
  const ordered = [...cells].sort(
    (p, q) =>
      (p.offset[0] - mx) * ux +
      (p.offset[1] - my) * uy -
      ((q.offset[0] - mx) * ux + (q.offset[1] - my) * uy),
  );
  // Fixture frame to stage: the mounting yaw; stage to canvas: y flips.
  const yaw = ((rotation?.[2] ?? 0) * Math.PI) / 180;
  return { ordered, angle: -(angle + yaw) };
}

/** A name for a new focus point that no existing one uses. */
export function nextFocusName(existing: Record<string, unknown>): string {
  let n = 1;
  while (`focus-${n}` in existing) n++;
  return `focus-${n}`;
}
