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

export interface BeatGrid {
  /** Beat times in seconds. */
  beats: number[];
  /** Index into `beats` of each measure's downbeat. */
  measure_starts: number[];
}

/**
 * How many beats a measure (1-based) holds, or null without a grid.
 *
 * `beats` is flat, so a beat past its measure's length is not out of range —
 * it is the next measure's, silently. The backend's `BeatGrid::beat_time`
 * refuses such a position, so anything that can write one (a stepper, a
 * drag) has to know where the measure ends. The last measure runs to the end
 * of the grid.
 */
export function beatsInMeasure(
  grid: BeatGrid | null | undefined,
  measure: number,
): number | null {
  if (!grid) return null;
  const start = grid.measure_starts[measure - 1];
  if (start === undefined) return null;
  const end = grid.measure_starts[measure] ?? grid.beats.length;
  return Math.max(0, end - start);
}

/**
 * Whether the grid ends inside this measure (1-based) — nothing follows its
 * last beat to interpolate a fractional position towards.
 */
function endsTheGrid(
  grid: BeatGrid | null | undefined,
  measure: number,
): boolean {
  if (!grid) return false;
  const nextStart = grid.measure_starts[measure];
  return nextStart === undefined || nextStart >= grid.beats.length;
}

/**
 * The largest beat that still lies inside a measure holding `beats` of them.
 *
 * Beat 4.5 of a 4/4 measure is inside it — between the last beat and the
 * next downbeat — but beat 5 is the downbeat itself and belongs to the
 * measure after. So the bound is one step short of `beats + 1`.
 *
 * The final measure is tighter: a fractional beat resolves by interpolating
 * towards the *next* grid beat, and past the end of the grid there is none.
 * `BeatGrid::beat_time` returns `None` there and the section disappears from
 * the resolved timeline, so the bound stops at the last beat that exists.
 *
 * The count is passed in because it is not always the grid's: where a
 * `tempo:` block exists it is the meter in effect, which is what the ruler
 * labels and what the grid was built from.
 */
export function maxBeatIn(
  grid: BeatGrid | null | undefined,
  measure: number,
  beats: number,
  step = 0.5,
): number {
  return Math.max(1, endsTheGrid(grid, measure) ? beats : beats + 1 - step);
}

/** `maxBeatIn` with the count read off the grid; null when it has none. */
export function maxBeatInMeasure(
  grid: BeatGrid | null | undefined,
  measure: number,
  step = 0.5,
): number | null {
  const beats = beatsInMeasure(grid, measure);
  return beats === null ? null : maxBeatIn(grid, measure, beats, step);
}

/**
 * Seconds at a (possibly fractional) beat of a measure, mirroring the
 * backend's `BeatGrid::beat_time`: measures and beats are 1-based, a
 * fractional beat interpolates between grid beats, and a beat past the end
 * of its measure is refused rather than read off the next measure.
 */
export function timeAtPosition(
  grid: BeatGrid | null | undefined,
  measure: number,
  beat: number,
): number | null {
  if (!grid || beat < 1) return null;
  const offset = beat - 1;
  const beats = beatsInMeasure(grid, measure);
  if (beats === null || Math.floor(offset) >= beats) return null;
  const base = grid.measure_starts[measure - 1];
  if (base === undefined) return null;
  const t0 = grid.beats[base + Math.floor(offset)];
  if (t0 === undefined) return null;
  const frac = offset - Math.floor(offset);
  if (frac === 0) return t0;
  // Interpolation needs a beat to head towards. Past the end of the grid the
  // backend has none and returns `None`; answering `t0` here would show a
  // time for a boundary the resolved timeline drops.
  const t1 = grid.beats[base + Math.floor(offset) + 1];
  return t1 === undefined ? null : t0 + (t1 - t0) * frac;
}

/**
 * The measure and (fractional) beat at a time in seconds. Times before the
 * grid clamp to its first beat, times past it to the last measure.
 */
export function positionAtTime(
  grid: BeatGrid | null | undefined,
  seconds: number,
): { measure: number; beat: number } | null {
  if (!grid || grid.beats.length === 0) return null;
  // The beat at or before this time.
  let index = 0;
  while (index + 1 < grid.beats.length && grid.beats[index + 1] <= seconds) {
    index++;
  }
  let measure = 0;
  while (
    measure + 1 < grid.measure_starts.length &&
    grid.measure_starts[measure + 1] <= index
  ) {
    measure++;
  }
  const start = grid.beats[index];
  const next = grid.beats[index + 1];
  const span = next === undefined ? 0 : next - start;
  const frac =
    span > 0 ? Math.max(0, Math.min(1, (seconds - start) / span)) : 0;
  return {
    measure: measure + 1,
    beat: index - grid.measure_starts[measure] + 1 + frac,
  };
}

/** The song time of a measure's downbeat, or null when it is off the grid. */
export function measureTime(
  grid: BeatGrid | null | undefined,
  measure: number,
): number | null {
  return timeAtPosition(grid, measure, 1);
}

/** "1:23.456" — the format the position readout shows and accepts. */
export function formatSeconds(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds - minutes * 60;
  return `${minutes}:${rest.toFixed(3).padStart(6, "0")}`;
}

/** "1:23", for tick labels where milliseconds would be noise. */
export function formatSecondsShort(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds - minutes * 60);
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

/** Parses "1:23.456", "83.456" or "83"; null when it is not a time. */
export function parseSeconds(raw: string): number | null {
  const match = /^(?:(\d+):)?(\d+(?:[.,]\d+)?)$/.exec(raw.trim());
  if (!match) return null;
  const minutes = match[1] ? parseInt(match[1]) * 60 : 0;
  return minutes + parseFloat(match[2].replace(",", "."));
}
