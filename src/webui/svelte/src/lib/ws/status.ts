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

import { derived, writable } from "svelte/store";
import { wsConnected } from "./stores";
import { uiOutdated, type ServerBuild } from "../buildCheck";

export interface SubsystemStatus {
  status:
    | "connected"
    | "initializing"
    | "failed"
    | "not_connected"
    | "not_configured";
  name: string | null;
  /** Why init gave up. Present only for `failed`. */
  error?: string | null;
}

export interface ControllerStatus {
  kind: string;
  status: string;
  detail: string | null;
  error: string | null;
}

export interface StatusData {
  build: {
    version: string;
    git_hash: string;
    build_time: string;
    /** The UI the server serves; null when its build stamped none. */
    ui_build?: string | null;
  };
  hardware: {
    init_done: boolean;
    hostname: string | null;
    profile: string | null;
    audio: SubsystemStatus;
    midi: SubsystemStatus;
    dmx: SubsystemStatus;
    trigger: SubsystemStatus;
    /** Whether the current venue registered. A venue fails whole when one
     *  fixture cannot be driven (a missing type, a bad mode), and then no
     *  fixture lights; `error` names the fixture and why. Absent from an
     *  older server. */
    lighting_venue?: LightingVenueStatus;
  };
  controllers: ControllerStatus[];
}

export interface LightingVenueStatus {
  name: string | null;
  status: "ok" | "failed" | "none";
  error: string | null;
}

/** Worst-case across all required subsystems. */
export type Health = "ok" | "warn" | "error" | "unknown";

export const statusStore = writable<StatusData | null>(null);

/** The server's build as first seen by this page. */
let firstSeen: ServerBuild | null = null;
statusStore.subscribe((s) => {
  if (s && !firstSeen) firstSeen = { ...s.build };
});

/** The UI this bundle was built as; empty for a build that stamped none. */
const UI_BUILD: string = import.meta.env.VITE_MTRACK_UI_BUILD ?? "";

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

let pollHandle: ReturnType<typeof setInterval> | null = null;

async function fetchStatus() {
  try {
    const res = await fetch("/api/status");
    if (!res.ok) return;
    const data = (await res.json()) as StatusData;
    statusStore.set(data);
  } catch {
    // Fetch failures are tolerated; the WS connect indicator already
    // surfaces total disconnects via the disconnect banner.
  }
}

function startPolling() {
  if (pollHandle !== null) return;
  void fetchStatus();
  pollHandle = setInterval(fetchStatus, 5000);
}

function stopPolling() {
  if (pollHandle === null) return;
  clearInterval(pollHandle);
  pollHandle = null;
}

// Poll while the WebSocket is up. When it drops we keep the last known
// snapshot but stop hammering /api/status.
wsConnected.subscribe((connected) => {
  if (connected) startPolling();
  else stopPolling();
});

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
