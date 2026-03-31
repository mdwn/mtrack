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

function playbackWithSections(
  wsId: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    type: "playback",
    is_playing: true,
    elapsed_ms: 3000,
    song_name: "Test Song Beta",
    song_duration_ms: 240000,
    playlist_name: "setlist",
    playlist_position: 1,
    playlist_songs: ["Test Song Alpha", "Test Song Beta"],
    tracks: [],
    available_playlists: ["all_songs", "setlist"],
    persisted_playlist_name: "setlist",
    locked: false,
    available_sections: [
      { name: "verse", start_measure: 1, end_measure: 4 },
      { name: "chorus", start_measure: 5, end_measure: 8 },
    ],
    active_section: null,
    ...overrides,
  };
}

test.describe("Section Loop Controls", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    wsId = `section-loop-${++testCounter}-${Date.now()}`;

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
    await expect(page.locator(".playback-song")).toContainText(
      "Test Song Alpha",
    );
  });

  test("clicking section button sends LoopSection gRPC call", async ({
    page,
  }) => {
    // Put into playing state with sections.
    await sendWsMessage(page, wsId, playbackWithSections(wsId));
    await expect(page.locator(".section-controls")).toBeVisible();

    // Click verse section button and wait for gRPC call.
    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("LoopSection"),
    );
    await page.getByRole("button", { name: "verse" }).click();
    await requestPromise;
  });

  test("clicking Stop Loop sends StopSectionLoop gRPC call", async ({
    page,
  }) => {
    // Put into playing state with an active section loop.
    await sendWsMessage(
      page,
      wsId,
      playbackWithSections(wsId, {
        active_section: { name: "verse", start_ms: 0, end_ms: 8000 },
      }),
    );
    await expect(page.getByRole("button", { name: "Stop Loop" })).toBeVisible();

    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("StopSectionLoop"),
    );
    await page.getByRole("button", { name: "Stop Loop" }).click();
    await requestPromise;
  });

  test("section buttons re-appear after stopping a loop", async ({ page }) => {
    // Start with active loop.
    await sendWsMessage(
      page,
      wsId,
      playbackWithSections(wsId, {
        active_section: { name: "verse", start_ms: 0, end_ms: 8000 },
      }),
    );
    await expect(page.locator(".section-active")).toBeVisible();

    // Simulate stop — server clears active section.
    await sendWsMessage(
      page,
      wsId,
      playbackWithSections(wsId, { active_section: null }),
    );
    await expect(page.locator(".section-active")).not.toBeVisible();
    await expect(page.getByRole("button", { name: "verse" })).toBeVisible({
      timeout: 10000,
    });
    await expect(page.getByRole("button", { name: "chorus" })).toBeVisible();
  });

  test("active section shows name", async ({ page }) => {
    await sendWsMessage(
      page,
      wsId,
      playbackWithSections(wsId, {
        active_section: { name: "chorus", start_ms: 8000, end_ms: 16000 },
      }),
    );
    await expect(page.locator(".section-active")).toContainText("chorus");
  });

  test("section controls hidden when song has no sections", async ({
    page,
  }) => {
    await sendWsMessage(page, wsId, {
      type: "playback",
      is_playing: true,
      elapsed_ms: 1000,
      song_name: "Test Song Alpha",
      song_duration_ms: 180000,
      playlist_name: "setlist",
      playlist_position: 0,
      playlist_songs: ["Test Song Alpha", "Test Song Beta"],
      tracks: [],
      available_playlists: ["all_songs", "setlist"],
      persisted_playlist_name: "setlist",
      locked: false,
      available_sections: [],
      active_section: null,
    });

    await expect(page.locator(".section-controls")).not.toBeVisible();
  });
});
