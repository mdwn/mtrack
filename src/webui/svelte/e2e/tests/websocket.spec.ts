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

test.describe("WebSocket Integration", () => {
  test("connection indicator shows connected after WS connects", async ({
    page,
  }) => {
    await page.goto("/#/");
    await expect(page.locator(".status-indicator.connected")).toBeVisible();
  });

  test("playback store populates from WS message", async ({ page }) => {
    await page.goto("/#/");
    // Playback data comes from WebSocket, not REST
    await expect(page.locator(".playback-song")).toContainText(
      "Test Song Alpha",
    );
    await expect(page.locator(".playback-status")).toContainText(/stopped/i);
  });

  test("waveform data loads from WS message", async ({ page }) => {
    await page.goto("/#/");
    // Waveform data arrives via WebSocket — verify tracks card has track rows
    // (canvas rendering depends on waveform data being present)
    await expect(page.locator(".track-row").first()).toBeVisible();
    await expect(page.locator(".track-waveform").first()).toBeVisible();
  });

  test("playlist songs populate from WS message", async ({ page }) => {
    await page.goto("/#/");
    const songs = page.locator(".playlist-songs li");
    await expect(songs).toHaveCount(2);
  });

  test("available playlists populate from WS message", async ({ page }) => {
    await page.goto("/#/");
    const options = page.locator(".playlist-select option");
    await expect(options).toHaveCount(2);
  });

  test("track info populates from WS message", async ({ page }) => {
    await page.goto("/#/");
    const tracks = page.locator(".track-row");
    await expect(tracks).toHaveCount(3);

    // Track names
    await expect(tracks.nth(0)).toContainText("kick");
    await expect(tracks.nth(1)).toContainText("snare");
    await expect(tracks.nth(2)).toContainText("bass");
  });

  test("track rows show output channel info", async ({ page }) => {
    await page.goto("/#/");
    const firstTrack = page.locator(".track-row").first();
    await expect(firstTrack.locator(".track-channels")).toBeVisible();
  });
});
