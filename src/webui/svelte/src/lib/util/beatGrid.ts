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
 * The largest beat that still lies inside its measure, for a given step.
 *
 * Beat 4.5 of a 4/4 measure is inside it — between the last beat and the
 * next downbeat — but beat 5 is the downbeat itself and belongs to the
 * measure after. So the bound is one step short of `beats + 1`.
 */
export function maxBeatInMeasure(
  grid: BeatGrid | null | undefined,
  measure: number,
  step = 0.5,
): number | null {
  const beats = beatsInMeasure(grid, measure);
  return beats === null ? null : Math.max(1, beats + 1 - step);
}
