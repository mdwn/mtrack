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

// The stage-only page: the web UI's stage plot with nothing around it, for a
// host that has no player to talk HTTP to. The phone app shows it in a web
// view and hands it the gRPC stage stream (`WatchStage`) it gets over
// Bluetooth. There is no websocket, no router and no REST call here: the
// page draws what it is given through `window.mtrackStage`, and nothing
// else. Built by `make build-stage-ui` into dist-stage/.
//
// THE HOST CONTRACT (also design/bluetooth_light_view_design.md §8.1)
//
// The page to the host's hand, on `window.mtrackStage`:
//
//   ready: boolean
//     True once the modules and the WebAssembly rules are loaded and the
//     page can take updates.
//
//   update(base64: string): void
//     One serialized player.v1.StageUpdate, base64 (standard alphabet,
//     padded). Updates given before `ready` are queued and applied in
//     order. A frame is read against the layout most recently given; a
//     frame for any other layout is dropped and counted. Bytes that do not
//     decode are counted as an error and reported to the host; the page
//     keeps what it was showing.
//
//   reset(): void
//     Forget the layout and all state: the stream ended or the player
//     changed. Shows the empty state. Drops anything still queued. The
//     counters are kept.
//
//   setTheme(theme: string): void
//     'light' | 'dark' | 'system' (anything else is 'system'). 'system'
//     follows prefers-color-scheme, and is what the page starts as. To
//     have the right theme at first paint, the host may also load the page
//     as `stage.html?theme=light` or `?theme=dark`.
//
//   stats(): { applied: number, dropped: number, errors: number, fps: number }
//     Counters for the host's diagnostics: updates applied (layouts and
//     frames), frames dropped for a wrong layout id, decode errors, and
//     rendered frames a second (the page's animation frames, over the last
//     second).
//
//   retry3d(): void
//     Build the 3D view again from nothing, asking again for every file it
//     could not have: for after a model did not load. Does nothing while
//     the plot is showing, or in a page without 3D.
//
// The page to the host, when `window.MtrackHost` exists (a Flutter
// JavaScript channel), as `window.MtrackHost.postMessage(JSON.stringify(m))`:
//
//   { type: 'ready' }            once, when `ready` turns true
//   { type: 'error', message }   a decode error, an uncaught error or an
//                                unhandled rejection
//   { type: 'view', view }       a page with 3D only: 'plot' or '3d', the
//                                view it starts in (right after `ready`)
//                                and then whenever the person switches
//
// With no `MtrackHost` the page works the same and says nothing.
//
// The page shows "No lights to show." until it holds a layout with venue
// fixtures in it, and again after `reset()`.
//
// `window.mtrackStage` exists as soon as this module has run, which is
// before `ready`: a host waits for the page to load (or for the `ready`
// message) before its first call, and may call `update` at once after that.
//
// 3D is in a build with MTRACK_STAGE_3D=1 only (vite.stage.config.ts),
// which offers the Plot | 3D switch. The page then starts in the plot, or
// in 3D when loaded as `stage.html?view=3d`. The 3D view asks the host for
// the player's lighting assets (rig models, meshes, scenery) at
// `api/lighting/assets/<path>`, relative to the page; the host answers
// from wherever it has them. A file it cannot answer is drawn without:
// a fixture as a plain box. The view takes touches: a drag turns it.

import { waitLocale } from "./lib/i18n";
import initRules from "./gen/rules/mtrack_rules";
import rulesUrl from "./gen/rules/mtrack_rules_bg.wasm?url";
import "./app.css";
import StageOnly from "./StageOnly.svelte";
import { mount } from "svelte";
import { base64Bytes, StageReader } from "./lib/stage/feed";
import { applyStageMetadata, applyStageState } from "./lib/ws/stores";

/** What the host calls. */
export interface MtrackStage {
  ready: boolean;
  update(base64: string): void;
  reset(): void;
  setTheme(theme: string): void;
  stats(): { applied: number; dropped: number; errors: number; fps: number };
  retry3d(): void;
}

declare global {
  interface Window {
    mtrackStage: MtrackStage;
    /** The host's ear, when there is a host (a Flutter JavaScript channel). */
    MtrackHost?: { postMessage(message: string): void };
  }
}

type View = "plot" | "3d";

type HostMessage =
  | { type: "ready" }
  | { type: "error"; message: string }
  | { type: "view"; view: View };

function tellHost(message: HostMessage): void {
  try {
    window.MtrackHost?.postMessage(JSON.stringify(message));
  } catch {
    // A host that cannot be told is no reason to stop drawing.
  }
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function reportError(what: string, error: unknown): void {
  tellHost({ type: "error", message: `${what}: ${describe(error)}` });
}

window.addEventListener("error", (e) =>
  reportError("uncaught error", e.error ?? e.message),
);
window.addEventListener("unhandledrejection", (e) =>
  reportError("unhandled rejection", e.reason),
);

// --- Theme: the web UI's own palette (`.nc--dark` on <html>), chosen by the
// host or the system. Not lib/theme.ts: that one remembers the choice in
// localStorage for the web UI's toggle, and this page has no toggle.
type Theme = "light" | "dark" | "system";
const asTheme = (theme: string | null): Theme =>
  theme === "light" || theme === "dark" ? theme : "system";
const query = new URLSearchParams(location.search);
let theme = asTheme(query.get("theme"));
const systemDark = window.matchMedia?.("(prefers-color-scheme: dark)");
function applyTheme(): void {
  const dark =
    theme === "dark" || (theme === "system" && !!systemDark?.matches);
  document.documentElement.classList.toggle("nc--dark", dark);
}
systemDark?.addEventListener("change", applyTheme);
applyTheme();

// --- The stream, into the stores the stage view reads: the same two
// writes the web UI's own stream reader (main.ts) ends in.
const reader = new StageReader({
  metadata: applyStageMetadata,
  state: applyStageState,
});
const counts = { applied: 0, dropped: 0, errors: 0 };
let fps = 0;
/** Updates given before the page was ready, oldest first. */
let queued: string[] = [];
/** The mounted page, once it is. */
let page: { retry3d(): void } | null = null;

function apply(base64: string): void {
  try {
    const outcome = reader.takeBytes(base64Bytes(base64));
    if (outcome === "dropped") counts.dropped++;
    else if (outcome !== "ignored") counts.applied++;
  } catch (error) {
    counts.errors++;
    reportError("stage update", error);
  }
}

const stage: MtrackStage = {
  ready: false,
  update(base64) {
    if (stage.ready) apply(base64);
    else queued.push(base64);
  },
  reset() {
    queued = [];
    reader.reset();
  },
  setTheme(next) {
    theme = asTheme(next);
    applyTheme();
  },
  stats: () => ({ ...counts, fps }),
  retry3d() {
    page?.retry3d();
  },
};
window.mtrackStage = stage;

// Rendered frames a second, for the host's diagnostics.
{
  let frames = 0;
  let since = performance.now();
  const tick = (now: number) => {
    frames++;
    if (now - since >= 1000) {
      fps = Math.round((frames * 1000) / (now - since));
      frames = 0;
      since = now;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

async function start(): Promise<void> {
  const target = document.getElementById("app")!;
  try {
    await Promise.all([waitLocale(), initRules({ module_or_path: rulesUrl })]);
  } catch (error) {
    // Without the rules module no fixture has a colour: say so.
    reportError("could not load the player's rules module", error);
    target.textContent = "Could not load the player's rules module.";
    return;
  }
  const threeD = import.meta.env.VITE_STAGE_3D === "1";
  const view: View = threeD && query.get("view") === "3d" ? "3d" : "plot";
  page = mount(StageOnly, {
    target,
    props: {
      threeD,
      view,
      onViewChange: (next: View) => tellHost({ type: "view", view: next }),
    },
  });
  stage.ready = true;
  for (const base64 of queued.splice(0)) apply(base64);
  tellHost({ type: "ready" });
  if (threeD) tellHost({ type: "view", view });

  // A desktop with no phone: `?dev=replay` or `?dev=grpc` on the dev server
  // (stage-dev.ts). Not in the built page.
  if (import.meta.env.DEV) {
    const dev = query.get("dev");
    if (dev) void import("./stage-dev").then((m) => m.startDev(dev));
  }
}

void start();
