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

import type { BeatGrid } from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";

/**
 * The highest beat on a `step` grid that still resolves inside a measure
 * (1-based) holding `beats` of them, asked of the player's grid.
 *
 * Beat 4½ of a 4/4 measure is inside it — between the last beat and the next
 * downbeat — but beat 5 is the downbeat itself and belongs to the measure
 * after, so the search starts one step short of `beats + 1`. In the measure
 * the grid ends in there is no next beat to interpolate towards, and
 * `BeatGrid::beat_time` refuses the fractional positions there (the section
 * would vanish from the resolved timeline), so the search steps down until
 * the grid answers. Without a grid there is nothing to refuse.
 *
 * The count is passed in because it is not always the grid's: where a
 * `tempo:` block exists it is the meter in effect, which is what the ruler
 * labels and what the grid was built from.
 */
export function highestBeat(
  grid: BeatGrid | null | undefined,
  measure: number,
  beats: number,
  step = 0.5,
): number {
  let beat = beats + 1 - step;
  if (grid) {
    while (
      beat > 1 &&
      rules.grid_query(grid, { measure, beat }) === undefined
    ) {
      beat -= step;
    }
  }
  return Math.max(1, beat);
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
