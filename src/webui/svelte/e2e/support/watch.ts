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
// Pushing what the player's gRPC stream `Watch` carries to a test's page
// through the mock server.
//
// The page reads its playback, waveform, log lines, reloads and hardware
// status from `Watch`. The mock server answers that call with a gRPC-Web
// response it keeps open per page (keyed by the page's `wsId`, the id a spec
// gives its page in the URL, as for the stage) and writes each pushed message
// into as it comes: what ./watch-stream.ts encodes. Every helper here
// answers once the message is written, or fails: the mock server waits (up
// to 5 s) for the page's stream to open first.
import { expect, type Page } from "@playwright/test";
import type { WatchPush } from "./watch-stream";

export type { WatchPush } from "./watch-stream";

const SEND_WATCH = "http://127.0.0.1:3111/test/send-watch";
const WATCH_OPENING = "http://127.0.0.1:3111/test/watch-opening";

/** Posts `msg` for the page connected as `wsId`, or, with no `wsId`, for
 *  every page connected; how many streams it went to. */
async function post(
  page: Page,
  wsId: string | null,
  msg: WatchPush,
): Promise<number> {
  const res = await page.request.post(SEND_WATCH, {
    data: wsId === null ? msg : { ...msg, _wsId: wsId },
  });
  return (await res.json()).sent;
}

/** Sends `msg` to the page connected as `wsId`, and throws if it was not
 *  delivered: for a spec whose next step depends on the page having it. */
export async function sendWatch(
  page: Page,
  wsId: string,
  msg: WatchPush,
): Promise<void> {
  if ((await post(page, wsId, msg)) !== 1) {
    throw new Error(`Watch message not delivered for wsId ${wsId}`);
  }
}

/** Sends `msg` until it is delivered (10 s at most): for a page whose
 *  stream may open again (through vite's proxy) while the spec runs. */
export async function sendWatchRetrying(
  page: Page,
  wsId: string,
  msg: WatchPush,
): Promise<void> {
  await expect(async () => {
    expect(await post(page, wsId, msg)).toBe(1);
  }).toPass({ timeout: 10000 });
}

/** Sends `msg` and does not check it arrived: for screenshots, which look
 *  at whatever the page shows. */
export async function sendWatchUnchecked(
  page: Page,
  wsId: string,
  msg: WatchPush,
): Promise<void> {
  await post(page, wsId, msg);
}

/** Sends `msg` to every page connected, whatever its wsId, and does not
 *  check how many that was: for a page opened without a wsId. */
export async function broadcastWatch(
  page: Page,
  msg: WatchPush,
): Promise<void> {
  await post(page, null, msg);
}

/** Sets what the stream of the page that will connect as `wsId` opens with:
 *  `msg` takes the place of the mock's default of its `type` (the playback,
 *  the waveform, the log lines or the hardware status), for every stream the
 *  page opens, until the mock server is restarted. Call it before the page
 *  opens (`page.goto`); it is not sent to a stream already open (use
 *  {@link sendWatch} for that). Throws when the mock refuses it. */
export async function setWatchOpening(
  page: Page,
  wsId: string,
  msg: WatchPush,
): Promise<void> {
  const res = await page.request.post(WATCH_OPENING, {
    data: { ...msg, _wsId: wsId },
  });
  if (!res.ok()) throw new Error(`Watch opening not set for wsId ${wsId}`);
}
