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

import type { Status } from "../../gen/rules/mtrack_rules";
import { derived, writable } from "svelte/store";
import { uiOutdated, type ServerBuild } from "../buildCheck";

/** Worst-case across all required subsystems. */
export type Health = "ok" | "warn" | "error" | "unknown";

/** The player's build, hardware and controllers, as its `hardware` events
 *  last said them (lib/grpc/watch.ts); null before the first. */
export const statusStore = writable<Status | null>(null);

/** The server's build as first seen by this page. */
let firstSeen: ServerBuild | null = null;
statusStore.subscribe((s) => {
  if (s && !firstSeen) firstSeen = { ...s.build };
});

/** The UI this bundle was built as; empty for a build that stamped none. */
const UI_BUILD: string = import.meta.env?.VITE_MTRACK_UI_BUILD ?? "";

/** This tab runs an older UI than the server now serves: reload for the
 *  new one. Never reloads by itself — someone may be mid-edit. */
export const uiUpdated = derived(statusStore, ($status) =>
  uiOutdated(UI_BUILD, firstSeen, $status?.build ?? null),
);

/** The current venue, when it did not load. */
export const venueFailure = derived(statusStore, ($status) => {
  const venue = $status?.hardware.lighting_venue;
  return venue?.status === "failed" ? venue : null;
});

/** The fixture test that is sending to the lights, when one is. */
export const testOutput = derived(
  statusStore,
  ($status) => $status?.hardware.test_output ?? null,
);

/**
 * Derives the worst-case health across required subsystems.
 *
 * - Audio is always required.
 * - MIDI / DMX are required if the active profile has them configured
 *   (i.e. they have a `name`). A song-aware refinement (only required if
 *   the active song uses them) is a follow-up — for now, "configured but
 *   not connected" surfaces as amber.
 * - Trigger is never required.
 */
export const healthStore = derived(statusStore, ($status): Health => {
  if (!$status) return "unknown";

  const subs = $status.hardware;
  if (!subs.init_done) return "warn";

  // Audio is always required — its status is the floor.
  if (subs.audio.status === "not_connected" || subs.audio.status === "failed")
    return "error";

  // MIDI / DMX are required if configured. `failed` needs no `name` check: a
  // subsystem only fails if it was configured, and a failed one has no name to
  // report — testing for one would silently downgrade it to healthy.
  for (const sub of [subs.midi, subs.dmx] as const) {
    if (sub.status === "failed") return "error";
    if (sub.name && sub.status === "not_connected") return "error";
  }

  // Initializing or any controller in error → amber.
  for (const sub of [subs.audio, subs.midi, subs.dmx] as const) {
    if (sub.status === "initializing") return "warn";
  }
  // A venue that did not load lights nothing.
  if (subs.lighting_venue?.status === "failed") return "error";

  if ($status.controllers.some((c) => c.status !== "running")) return "warn";

  return "ok";
});
