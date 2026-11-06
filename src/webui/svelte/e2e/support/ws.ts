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

// Pushing WebSocket messages to a test's page through the mock server.
//
// The mock server's /test/send-ws waits (up to 5 s) for the page's socket to
// connect and finish its opening burst, and answers `sent: 1` once it has
// delivered the message, `sent: 0` when it could not.
import { expect, type Page } from "@playwright/test";

const SEND_WS = "http://127.0.0.1:3111/test/send-ws";

async function post(page: Page, wsId: string, msg: object): Promise<number> {
  const res = await page.request.post(SEND_WS, {
    data: { ...msg, _wsId: wsId },
  });
  return (await res.json()).sent;
}

/** Sends `msg` to the page connected as `wsId`, and throws if it was not
 * delivered: for a spec whose next step depends on the page having it. */
export async function sendWs(
  page: Page,
  wsId: string,
  msg: object,
): Promise<void> {
  if ((await post(page, wsId, msg)) !== 1) {
    throw new Error(`WS message not delivered for wsId ${wsId}`);
  }
}

/** Sends `msg` until it is delivered (10 s at most): for a page whose
 * socket may reconnect (through vite's proxy) while the spec runs. */
export async function sendWsRetrying(
  page: Page,
  wsId: string,
  msg: object,
): Promise<void> {
  await expect(async () => {
    expect(await post(page, wsId, msg)).toBe(1);
  }).toPass({ timeout: 10000 });
}

/** Sends `msg` and does not check it arrived: for screenshots, which look
 * at whatever the page shows. */
export async function sendWsUnchecked(
  page: Page,
  wsId: string,
  msg: object,
): Promise<void> {
  await post(page, wsId, msg);
}
