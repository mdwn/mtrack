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

import type { TempoChangeConfig } from "../api/songs";

/** The beat a tempo change sits on; absent means the downbeat. */
export function changeBeat(change: TempoChangeConfig): number {
  return change.beat ?? 1;
}

/**
 * Tempo changes ordered by position.
 *
 * `TempoConfig::to_tempo_map` requires strictly ascending (measure, beat), and
 * every save runs through it, so an array written in click order comes back as
 * an HTTP 400 the UI has no way to repair. Editors sort on the way out instead.
 */
export function sortTempoChanges(
  changes: TempoChangeConfig[],
): TempoChangeConfig[] {
  return [...changes].sort(
    (a, b) => a.measure - b.measure || changeBeat(a) - changeBeat(b),
  );
}

/** Whether a change other than `exceptIndex` already sits at this position. */
export function positionTaken(
  changes: TempoChangeConfig[],
  measure: number,
  beat: number,
  exceptIndex = -1,
): boolean {
  return changes.some(
    (c, i) =>
      i !== exceptIndex && c.measure === measure && changeBeat(c) === beat,
  );
}
