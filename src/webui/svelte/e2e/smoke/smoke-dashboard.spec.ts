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
import { ALL_SONGS, waitForPlaybackState } from "./helpers.js";

test.describe("Smoke: Dashboard", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await waitForPlaybackState(page);
  });

  test("page loads and shows mtrack branding", async ({ page }) => {
    await expect(page.locator(".nav-brand")).toContainText("mtrack");
    await expect(page).toHaveTitle(/mtrack/);
  });

  test("WebSocket connects and delivers playback state", async ({ page }) => {
    // Song name should appear from real WebSocket data.
    const songName = page.locator(".playback-song");
    await expect(songName).toBeVisible();
    // Should be one of our example songs.
    const text = await songName.textContent();
    expect(ALL_SONGS).toContain(text!.trim());
  });

  test("playlist shows songs", async ({ page }) => {
    const playlistSongs = page.locator(".playlist-songs li");
    await expect(playlistSongs.first()).toBeVisible();

    // Should have songs listed.
    const count = await playlistSongs.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("transport controls are visible and enabled", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Play" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Play" })).toBeEnabled();
    await expect(
      page.getByRole("button", { name: "Next", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Prev", exact: true }),
    ).toBeVisible();
  });

  test("tracks from current song are displayed", async ({ page }) => {
    const trackRows = page.locator(".track-row");
    await expect(trackRows.first()).toBeVisible({ timeout: 10000 });

    const count = await trackRows.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("progress bar is visible", async ({ page }) => {
    await expect(page.locator(".progress-bar")).toBeVisible();
  });

  test("track count badge is shown", async ({ page }) => {
    await expect(page.locator(".track-count")).toBeVisible();
  });
});
