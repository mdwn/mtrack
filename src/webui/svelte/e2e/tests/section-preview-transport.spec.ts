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
  const res = await page.request.post("http://127.0.0.1:3111/test/send-ws", {
    data: { ...msg, _wsId: wsId },
  });
  const body = await res.json();
  if (body.sent !== 1) {
    throw new Error(`WS message not delivered for wsId ${wsId}`);
  }
}

/** Playback state for Test Song Beta (the mock song with a beat grid:
 * 16 measures at 120 BPM in 4/4 → beat every 0.5s, measure every 2s). */
function playbackState(overrides: Record<string, unknown> = {}) {
  return {
    type: "playback",
    is_playing: false,
    elapsed_ms: 5000,
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

test.describe("Section editor preview transport", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    // The worker index keeps ids unique across parallel workers, whose
    // per-worker counters can otherwise collide within the same millisecond.
    wsId = `preview-transport-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;

    // A well-formed grpc-web unary reply: an empty message frame followed by
    // a trailers frame. A bare empty body makes the client's response parsing
    // reject, which silently aborts call chains like pause's stop-then-seek.
    const trailers = Buffer.from("grpc-status: 0\r\n");
    const grpcWebBody = Buffer.concat([
      Buffer.from([0x00, 0, 0, 0, 0]),
      Buffer.from([0x80, 0, 0, 0, trailers.length]),
      trailers,
    ]);
    await page.route("**/player.v1.PlayerService/**", async (route) => {
      await route.fulfill({
        status: 200,
        headers: { "content-type": "application/grpc-web+proto" },
        body: grpcWebBody,
      });
    });

    await page.goto(`/?wsId=${wsId}#/songs/Test%20Song%20Beta/sections`);
    await expect(page.locator(".section-timeline-editor")).toBeVisible();
  });

  test("transport renders; stop is disabled while another song is current", async ({
    page,
  }) => {
    const buttons = page.locator(".preview-btn");
    await expect(buttons).toHaveCount(2);
    // The initial mock state has Test Song Alpha current, so Beta's preview
    // shows a plain play button and a disabled stop.
    await expect(buttons.nth(0)).toBeEnabled();
    await expect(buttons.nth(0)).not.toHaveClass(/preview-btn--live/);
    await expect(buttons.nth(1)).toBeDisabled();
  });

  test("play on a non-current song starts it from the top via PlaySongFrom", async ({
    page,
  }) => {
    const requestPromise = page.waitForRequest((req) =>
      req.url().includes("/PlaySongFrom"),
    );
    await page.locator(".preview-btn").first().click();
    await requestPromise;
  });

  test("playhead and musical readout appear when the song is current", async ({
    page,
  }) => {
    await sendWsMessage(page, wsId, playbackState());

    // Stopped at 5.0s → measure 3, beat 3 of the 120 BPM 4/4 grid.
    await expect(page.locator(".playhead")).toBeVisible();
    await expect(page.locator(".playhead-info__pos")).toHaveText("m3.3");
    await expect(page.locator(".playhead-info")).toContainText("120");
  });

  test("pause on the playing current song fires Stop then Seek", async ({
    page,
  }) => {
    await sendWsMessage(page, wsId, playbackState({ is_playing: true }));

    const playPause = page.locator(".preview-btn").first();
    await expect(playPause).toHaveClass(/preview-btn--live/);

    const stopPromise = page.waitForRequest((req) =>
      req.url().includes("/Stop"),
    );
    const seekPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await playPause.click();
    await stopPromise;
    await seekPromise;
  });

  test("stop rewinds the stopped current song to the top", async ({ page }) => {
    await sendWsMessage(page, wsId, playbackState());

    const stopBtn = page.locator(".preview-btn").nth(1);
    await expect(stopBtn).toBeEnabled();

    const seekPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await stopBtn.click();
    await seekPromise;
  });

  test("arrow keys on the focused playhead seek the preview", async ({
    page,
  }) => {
    await sendWsMessage(page, wsId, playbackState());

    const playhead = page.locator(".playhead");
    await playhead.focus();

    const seekPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await page.keyboard.press("ArrowRight");
    await seekPromise;
  });
});
