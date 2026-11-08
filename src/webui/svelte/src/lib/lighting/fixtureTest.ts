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
 * Testing a fixture from its page: send a colour, a level, a strobe or a
 * pose to one fixture at an address, through the engine, to see that the
 * light answers. The server resolves the controls exactly as a show would
 * and holds them for a few seconds; the page keeps them alive while Send is
 * on, and releases them when it stops.
 */

import type {
  FrameChannel,
  TestAnswer,
  TestMode,
  TestOptions,
  TestRequest,
} from "../../gen/rules/mtrack_rules";
import { rules } from "../rules";
import { dirParams } from "../api/config";

/** The controls the panel holds, whatever the mode can take of them. */
export interface TestControls {
  color: string;
  dimmer: number;
  /** Hz, or null for off. */
  strobe: number | null;
  pan: number;
  tilt: number;
  levels: Record<string, number>;
}

const BASE = "/api/lighting/fixture-test";

/** A refusal: the HTTP status and the server's reason, when it gave one. */
export class TestRefused extends Error {
  status: number;
  reason: string | null;

  constructor(message: string, status: number, reason: string | null) {
    super(message);
    this.status = status;
    this.reason = reason;
  }
}

/** Full white at rest: what Send shows before anything else is touched,
 *  so a healthy fixture visibly answers. */
export function startingControls(mode: TestMode | null): TestControls {
  const levels: Record<string, number> = {};
  for (const name of mode?.controls?.levels ?? []) levels[name] = 0;
  return {
    color: "#ffffff",
    dimmer: 1,
    strobe: null,
    pan: 0,
    tilt: 0,
    levels,
  };
}

/** The request for these controls in this mode: only what the mode can do
 *  is sent. */
export function testBody(
  fixtureType: string,
  mode: TestMode,
  universe: number,
  address: number,
  controls: TestControls,
  raw: Record<number, number>,
): TestRequest {
  const c = mode.controls;
  const out: Record<string, string | number | null> = {};
  if (c?.color) out.color = controls.color;
  if (c?.dimmer) out.dimmer = controls.dimmer;
  if (c?.strobe) out.strobe = controls.strobe;
  if (c?.pan) out.pan = controls.pan;
  if (c?.tilt) out.tilt = controls.tilt;
  for (const name of c?.levels ?? []) out[name] = controls.levels[name] ?? 0;
  const rawOut: Record<string, number> = {};
  for (const [offset, value] of Object.entries(raw)) rawOut[offset] = value;
  return {
    fixture_type: fixtureType,
    mode: mode.name,
    universe,
    address,
    controls: out,
    raw: rawOut,
  };
}

/** The last address the fixture takes (`PatchSpan::last`); a mode that
 *  writes nothing is shown at its own address. */
export function lastAddress(address: number, footprint: number): number {
  return (
    rules.patch_last({ fixture: "", universe: 1, address, footprint }) ??
    address
  );
}

/** The named values the 3D view's live inputs take (0–255), from a frame. */
export function frameChannels(frame: FrameChannel[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const ch of frame) {
    if (!(ch.name in out)) out[ch.name] = ch.value;
  }
  return out;
}

async function refusal(res: Response, fallback: string): Promise<TestRefused> {
  const body: { error?: string; reason?: string } = await res
    .json()
    .catch(() => ({}));
  return new TestRefused(
    body.error || `${fallback}: ${res.status}`,
    res.status,
    body.reason ?? (res.status === 423 ? "locked" : null),
  );
}

export async function fetchTestOptions(
  fixtureType: string,
  dir?: string,
  venuesDir?: string,
): Promise<TestOptions> {
  const query = dirParams(dir, venuesDir, { fixture_type: fixtureType });
  const res = await fetch(`${BASE}/options${query}`);
  if (!res.ok) throw await refusal(res, "Failed to read the test options");
  return res.json();
}

export async function sendTest(
  body: TestRequest,
  dir?: string,
  venuesDir?: string,
): Promise<TestAnswer> {
  const res = await fetch(`${BASE}${dirParams(dir, venuesDir)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw await refusal(res, "Failed to send");
  return res.json();
}

/** Releases the test output. `keepalive` lets it go out as the page
 *  closes. */
export async function releaseTest(keepalive = false): Promise<void> {
  await fetch(BASE, { method: "DELETE", keepalive });
}

/**
 * Sends at most one request per `interval` ms, always ending on the latest
 * state (a trailing send), one at a time. `run` reads the state when it
 * goes, so a burst of slider moves costs a few requests, not one each.
 */
export class Throttle {
  private pending = false;
  private inflight = false;
  private last = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private run: () => Promise<void>;
  private interval: number;

  constructor(run: () => Promise<void>, interval = 50) {
    this.run = run;
    this.interval = interval;
  }

  request(): void {
    this.pending = true;
    void this.pump();
  }

  cancel(): void {
    this.pending = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private async pump(): Promise<void> {
    if (this.inflight || !this.pending) return;
    const wait = this.interval - (Date.now() - this.last);
    if (wait > 0) {
      if (!this.timer)
        this.timer = setTimeout(() => {
          this.timer = null;
          void this.pump();
        }, wait);
      return;
    }
    this.pending = false;
    this.inflight = true;
    this.last = Date.now();
    try {
      await this.run();
    } finally {
      this.inflight = false;
      void this.pump();
    }
  }
}
