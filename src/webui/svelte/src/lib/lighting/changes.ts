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
 * "This venue (or fixture type) changed on disk": one signal, raised by the
 * API client after every successful write, that every view of the data
 * re-reads on. A view keyed only on a venue's name never reloads when the
 * name stays the same — a saved venue then showed its old file until the
 * user clicked away and back.
 */

import { writable } from "svelte/store";

/** A venue file changed: `name` is the venue, or null for "any venue" (a
 *  rename or a type change can touch several). `seq` makes every change a
 *  new value, so the same name twice still notifies. */
export interface VenueChange {
  seq: number;
  name: string | null;
}

export const venueChanges = writable<VenueChange>({ seq: 0, name: null });

/** Bumped when a fixture type file changed (saved, renamed, deleted,
 *  imported): footprints, modes and type lists are stale. */
export const fixtureTypeChanges = writable(0);

let seq = 0;

export function venueChanged(name: string | null): void {
  venueChanges.set({ seq: ++seq, name });
}

export function fixtureTypesChanged(): void {
  fixtureTypeChanges.update((n) => n + 1);
}

/** Whether a change concerns the venue a view shows. */
export function concerns(change: VenueChange, venue: string | null): boolean {
  return change.seq > 0 && (change.name === null || change.name === venue);
}
