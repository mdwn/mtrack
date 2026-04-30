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

import { writable, get } from "svelte/store";
import { showConfirm } from "./dialog.svelte";

interface GuardEntry {
  /** Returns true while the editor has unsaved changes. */
  isDirty: () => boolean;
  /** Optional override for the confirm-discard message. */
  message?: string;
}

const guards = writable<Map<symbol, GuardEntry>>(new Map());

/**
 * Register a dirty-state probe with the global navigation guard.
 *
 * Call this in a Svelte `$effect` and return the unregister function so the
 * editor's dirty state stops blocking navigation when the component unmounts.
 *
 * ```svelte
 * $effect(() => registerDirtyGuard(
 *   () => anyDirty,
 *   $t("songs.detail.discardUnsaved"),
 * ));
 * ```
 */
export function registerDirtyGuard(
  isDirty: () => boolean,
  message?: string,
): () => void {
  const id = Symbol("dirtyGuard");
  guards.update((m) => {
    const next = new Map(m);
    next.set(id, { isDirty, message });
    return next;
  });
  return () => {
    guards.update((m) => {
      const next = new Map(m);
      next.delete(id);
      return next;
    });
  };
}

/**
 * Returns true if any registered editor reports unsaved changes.
 */
export function hasDirty(): boolean {
  for (const entry of get(guards).values()) {
    if (entry.isDirty()) return true;
  }
  return false;
}

/**
 * If anything is dirty, prompt the user to confirm discarding. Returns true
 * when navigation should proceed (clean, or user accepted the discard).
 */
export async function confirmNavigation(): Promise<boolean> {
  for (const entry of get(guards).values()) {
    if (!entry.isDirty()) continue;
    const ok = await showConfirm(entry.message ?? "Discard unsaved changes?", {
      danger: true,
    });
    if (!ok) return false;
  }
  return true;
}
