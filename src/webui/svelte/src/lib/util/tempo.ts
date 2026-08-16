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

/**
 * Ordered changes with repeated positions collapsed, and how many went.
 *
 * `to_tempo_map` rejects a repeat as firmly as it rejects a reversal, so a
 * source that anchors two changes to one beat — or two time-anchored ones that
 * snap onto the same beat — has to lose one before the song can be saved. The
 * last of a run wins: `TempoMap::new` sorts stably and applies changes in
 * order, so that is the one that was in effect anyway.
 *
 * The editors guard positions with `positionTaken` as they are typed; this is
 * for changes arriving in bulk, where there is nothing to guard.
 */
export function orderTempoChanges(changes: TempoChangeConfig[]): {
  changes: TempoChangeConfig[];
  duplicates: number;
} {
  const sorted = sortTempoChanges(changes);
  const kept = sorted.filter((c, i) => {
    const next = sorted[i + 1];
    return (
      next === undefined ||
      c.measure !== next.measure ||
      changeBeat(c) !== changeBeat(next)
    );
  });
  return { changes: kept, duplicates: sorted.length - kept.length };
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
