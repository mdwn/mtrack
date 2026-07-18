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

let testCounter = 0;

async function sendWsMessage(
  page: import("@playwright/test").Page,
  wsId: string,
  msg: object,
) {
  await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
}

function playbackState(overrides: Record<string, unknown> = {}) {
  return {
    type: "playback",
    is_playing: true,
    elapsed_ms: 30000,
    song_name: "Test Song Beta",
    song_duration_ms: 240000,
    playlist_name: "setlist",
    playlist_position: 1,
    playlist_songs: ["Test Song Alpha", "Test Song Beta"],
    tracks: [],
    available_playlists: ["all_songs", "setlist"],
    persisted_playlist_name: "setlist",
    locked: false,
    available_sections: [],
    active_section: null,
    ...overrides,
  };
}

test.describe("Seek", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `seek-${++testCounter}-${Date.now()}`;

    await page.route("**/player.v1.PlayerService/**", async (route) => {
      await route.fulfill({
        status: 200,
        headers: {
          "content-type": "application/grpc-web+proto",
          "grpc-status": "0",
        },
        body: Buffer.alloc(0),
      });
    });

    await page.goto(`/?wsId=${wsId}#/`);
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("clicking the progress bar sends a Seek gRPC call", async ({ page }) => {
    await sendWsMessage(page, wsId, playbackState());

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
    await sendWsMessage(page, wsId, playbackState());

    const scrub = page.locator(".scrub");
    await scrub.focus();

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await page.keyboard.press("ArrowRight");
    await requestPromise;
  });

  test("pending seek position marker shows while stopped", async ({ page }) => {
    await sendWsMessage(
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
    await sendWsMessage(
      page,
      wsId,
      playbackState({ is_playing: true, pending_start_ms: null }),
    );
    await expect(page.locator(".scrub__pending")).not.toBeVisible();
  });
});
