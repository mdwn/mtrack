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
import { ALL_SONGS, waitForWebSocket } from "./helpers.js";

test.describe("Smoke: Song Browser", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs");
    await waitForWebSocket(page);
  });

  test("song browser lists all example songs", async ({ page }) => {
    // Wait for song rows to load.
    await expect(page.locator(".song-row").first()).toBeVisible({
      timeout: 10000,
    });

    // Every example song should appear somewhere on the page.
    for (const song of ALL_SONGS) {
      await expect(page.locator(".song-row", { hasText: song })).toBeVisible();
    }
  });

  test("clicking a song shows its detail page", async ({ page }) => {
    // Wait for songs to load.
    await expect(page.locator(".song-row").first()).toBeVisible({
      timeout: 10000,
    });

    // Click a song row.
    await page.locator(".song-row", { hasText: ALL_SONGS[0] }).click();

    // Should navigate to song detail with tabs.
    await expect(page.getByText(ALL_SONGS[0])).toBeVisible({
      timeout: 10000,
    });
    await expect(page.locator(".tab").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("song detail shows track information", async ({ page }) => {
    // Navigate to a song with known tracks.
    await page.goto(`/#/songs/${encodeURIComponent("Another cool song")}`);

    // Wait for content.
    await expect(page.getByText("Another cool song")).toBeVisible({
      timeout: 10000,
    });

    // Track names appear in input fields on the detail page.
    // Check for the track name inputs containing expected values.
    await expect(page.locator('input[value="click"]')).toBeVisible({
      timeout: 10000,
    });
    await expect(page.locator('input[value="backing-track-l"]')).toBeVisible();
  });

  test("song browser has search functionality", async ({ page }) => {
    await expect(page.locator(".song-row").first()).toBeVisible({
      timeout: 10000,
    });

    // Search for a specific song.
    const searchInput = page.locator(".search-input");
    await expect(searchInput).toBeVisible();
    await searchInput.fill("cool");

    // Should filter results.
    await expect(
      page.locator(".song-row", { hasText: "cool song" }),
    ).toBeVisible();
  });
});
