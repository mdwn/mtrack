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
import { waitForWebSocket } from "./helpers.js";

test.describe("Smoke: Playlists", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/playlists");
    await waitForWebSocket(page);
  });

  test("playlists page loads and shows playlist list", async ({ page }) => {
    // Should show at least the "playlist" from the example data.
    await expect(page.locator(".playlist-list li").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("clicking a playlist shows its songs", async ({ page }) => {
    // Click on the first playlist.
    const playlistItem = page.locator(".playlist-list li").first();
    await expect(playlistItem).toBeVisible({ timeout: 10000 });
    await playlistItem.click();

    // Should show songs in the detail panel.
    await expect(page.locator(".song-list li").first()).toBeVisible({
      timeout: 10000,
    });
  });
});
