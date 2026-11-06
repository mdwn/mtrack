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
import { fromBinary } from "@bufbuild/protobuf";
import { SeekRequestSchema } from "../../src/gen/player/v1/player_pb";
import { sendWs } from "../support/ws";
import { playbackState as sharedPlaybackState } from "../support/playback";

let testCounter = 0;

/** Playback state for Test Song Beta (the mock song with a beat grid:
 * 16 measures at 120 BPM in 4/4 → beat every 0.5s, measure every 2s). */
function playbackState(
  overrides: Parameters<typeof sharedPlaybackState>[0] = {},
) {
  return sharedPlaybackState({ elapsed_ms: 5000, ...overrides });
}

test.describe("Section editor preview transport", () => {
  let wsId: string;

  test.beforeEach(async ({ page }) => {
    // The worker index keeps ids unique across parallel workers, whose
    // per-worker counters can otherwise collide within the same millisecond.
    wsId = `preview-transport-${test.info().parallelIndex}-${++testCounter}-${Date.now()}`;

    await stubPlayerService(page, { reply: "framed" });

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
    await sendWs(page, wsId, playbackState());

    // Stopped at 5.0s → measure 3, beat 3 of the 120 BPM 4/4 grid.
    await expect(page.locator(".playhead")).toBeVisible();
    await expect(page.locator(".playhead-info__pos")).toHaveText("m3.3");
    await expect(page.locator(".playhead-info")).toContainText("120");
  });

  test("pause on the playing current song fires Stop then Seek", async ({
    page,
  }) => {
    await sendWs(page, wsId, playbackState({ is_playing: true }));

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
    await sendWs(page, wsId, playbackState());

    const stopBtn = page.locator(".preview-btn").nth(1);
    await expect(stopBtn).toBeEnabled();

    const seekPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await stopBtn.click();
    await seekPromise;
  });

  test("arrow keys near the start clamp instead of seeking negative", async ({
    page,
  }) => {
    // Two seconds in, so ArrowLeft's five-second step would land at -3s. A
    // negative Duration is rejected by the backend and only logged, which
    // would make the key a silent no-op.
    await sendWs(page, wsId, playbackState({ elapsed_ms: 2000 }));

    const playhead = page.locator(".playhead");
    await playhead.focus();

    const request = page.waitForRequest((req) => req.url().includes("/Seek"));
    await page.keyboard.press("ArrowLeft");
    const body = (await request).postDataBuffer();
    if (!body) throw new Error("seek had no body");
    // grpc-web frames the message after a 5-byte header.
    const seek = fromBinary(SeekRequestSchema, body.subarray(5));
    expect(Number(seek.position?.seconds ?? 0n)).toBe(0);
    expect(seek.position?.nanos ?? 0).toBe(0);
  });

  test("arrow keys on the focused playhead seek the preview", async ({
    page,
  }) => {
    await sendWs(page, wsId, playbackState());

    const playhead = page.locator(".playhead");
    await playhead.focus();

    const seekPromise = page.waitForRequest((req) =>
      req.url().includes("/Seek"),
    );
    await page.keyboard.press("ArrowRight");
    await seekPromise;
  });

  test("the section dialog captures the playhead as a boundary", async ({
    page,
  }) => {
    // Stopped at 5.0s → measure 3, beat 3 of the 120 BPM 4/4 grid.
    await sendWs(page, wsId, playbackState());

    const firstBlock = page.locator(".section-block").first();
    const box = await firstBlock.boundingBox();
    if (!box) throw new Error("section block not found");
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);

    // The dialog shows the playhead position and captures it as the end
    // boundary of verse (m1–4), beat-precise.
    await expect(page.locator(".playhead-capture")).toContainText("m3.3");
    await page.getByRole("button", { name: "End here" }).click();
    await expect(page.locator(".range-note")).toHaveText("m1–3.3");
  });
});
