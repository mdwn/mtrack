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

import { test, expect } from "@playwright/test";
import { stubPlayerService } from "../support/grpc";
import { sendWatch } from "../support/watch";
import { playbackState as sharedPlaybackState } from "../support/playback";

let testCounter = 0;

function playbackState(
  overrides: Parameters<typeof sharedPlaybackState>[0] = {},
) {
  return sharedPlaybackState({
    is_playing: true,
    elapsed_ms: 30000,
    ...overrides,
  });
}

test.describe("Seek", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    // The worker index keeps ids unique across parallel workers, whose
    // per-worker counters can otherwise collide within the same millisecond.
    wsId = `seek-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;

    await stubPlayerService(page);

    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("clicking the progress bar sends a Seek gRPC call", async ({ page }) => {
    await sendWatch(page, wsId, playbackState());

    const scrub = page.locator(".scrub");
    await expect(scrub).toBeVisible();

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    // Click at 50% of the bar → seek to ~120s of the 240s song.
    const box = await scrub.boundingBox();
    if (!box) throw new Error("scrub bounding box unavailable");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await requestPromise;
  });

  test("arrow keys on the focused progress bar seek instead of navigating", async ({
    page,
  }) => {
    await sendWatch(page, wsId, playbackState());

    const scrub = page.locator(".scrub");
    await scrub.focus();

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await page.keyboard.press("ArrowRight");
    await requestPromise;
  });

  test("pending seek position marker shows while stopped", async ({ page }) => {
    await sendWatch(
      page,
      wsId,
      playbackState({
        is_playing: false,
        elapsed_ms: 0,
        pending_start_ms: 60000,
      }),
    );

    await expect(page.locator(".scrub__pending")).toBeVisible();

    // Marker disappears once playback starts (pending consumed).
    await sendWatch(
      page,
      wsId,
      playbackState({ is_playing: true, pending_start_ms: null }),
    );
    await expect(page.locator(".scrub__pending")).not.toBeVisible();
  });
});
