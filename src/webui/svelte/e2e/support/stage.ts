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

// Pushing the stage to a test's page through the mock server.
//
// The page reads the stage from the player's gRPC stream `WatchStage`. The
// mock server answers that call with a gRPC-Web response it keeps open per
// page (keyed by the page's `wsId`, as the Watch stream is) and writes each
// pushed message into as it comes: what ./stage-stream.ts encodes. Every
// helper here answers once the message is written, or fails: the mock
// server waits (up to 5 s) for the page's stream to open first.
import { expect, type Page } from "@playwright/test";
import type { StageMessage } from "./stage-stream";

export type {
  StageMessage,
  StageMetadataMessage,
  StageStateMessage,
} from "./stage-stream";

const SEND_STAGE = "http://127.0.0.1:3111/test/send-stage";

/** Posts `msg` for the page connected as `wsId`, or, with no `wsId`, for
 *  every page; how many streams it went to. */
async function post(
  page: Page,
  wsId: string | null,
  msg: StageMessage,
): Promise<number> {
  const res = await page.request.post(SEND_STAGE, {
    data: wsId === null ? msg : { ...msg, _wsId: wsId },
  });
  return (await res.json()).sent;
}

/** Puts `msg` on the stage of the page connected as `wsId`, and throws if
 *  it was not delivered: for a spec whose next step depends on the page
 *  having it. */
export async function sendStage(
  page: Page,
  wsId: string,
  msg: StageMessage,
): Promise<void> {
  if ((await post(page, wsId, msg)) !== 1) {
    throw new Error(`stage message not delivered for wsId ${wsId}`);
  }
}

/** [`sendStage`], retried (10 s at most): for a page whose stream may open
 *  again (through vite's proxy) while the spec runs. */
export async function sendStageRetrying(
  page: Page,
  wsId: string,
  msg: StageMessage,
): Promise<void> {
  await expect(async () => {
    expect(await post(page, wsId, msg)).toBe(1);
  }).toPass({ timeout: 10000 });
}

/** Puts `msg` on the stage and does not check it arrived: for screenshots,
 *  which look at whatever the page shows. */
export async function sendStageUnchecked(
  page: Page,
  wsId: string,
  msg: StageMessage,
): Promise<void> {
  await post(page, wsId, msg);
}

/** Puts `msg` on every page's stage, whatever its wsId, and does not check
 *  how many that was: for a page opened without a wsId. */
export async function broadcastStage(
  page: Page,
  msg: StageMessage,
): Promise<void> {
  await post(page, null, msg);
}

/** Opens `hash` with the stage already on the page. The stage arrives from
 *  its stream a moment after a page that shows it opens, and a page that
 *  loads on the stage's first layout loads twice; a spec that counts or
 *  stubs the page's reads goes through the dashboard first (until the mock's
 *  default stage has been applied) and then on to `hash` within the app. */
export async function openWithStage(page: Page, hash: string): Promise<void> {
  await page.goto("/#/");
  await expect(page.locator(".effects-card__chips .badge")).toHaveCount(2);
  await page.evaluate((h) => {
    window.location.hash = h;
  }, hash);
}
