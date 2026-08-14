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
import { waitForPlaybackState } from "./helpers.js";

test.describe("Smoke: Playback", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await waitForPlaybackState(page);
  });

  test.afterEach(async ({ page }) => {
    // Always stop playback after each test.
    const stopBtn = page.getByRole("button", { name: "Stop" });
    if (await stopBtn.isVisible().catch(() => false)) {
      await stopBtn.click();
      await expect(page.locator(".playback-status .stopped"))
        .toBeVisible({ timeout: 5000 })
        .catch(() => {});
    }
  });

  test("play starts playback and stop returns to stopped", async ({ page }) => {
    // Initial state: stopped.
    await expect(page.locator(".playback-status .stopped")).toBeVisible();

    // Click Play.
    await page.getByRole("button", { name: "Play" }).click();

    // Status should transition to playing.
    await expect(page.locator(".playback-status .playing")).toBeVisible({
      timeout: 10000,
    });

    // Stop button should appear.
    await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();

    // Wait a moment for elapsed time to advance.
    await page.waitForTimeout(1500);

    // Click Stop.
    await page.getByRole("button", { name: "Stop" }).click();
    await expect(page.locator(".playback-status .stopped")).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByRole("button", { name: "Play" })).toBeVisible();
  });

  test("next advances to a different song", async ({ page }) => {
    // Get current song name.
    const initialSong = await page.locator(".playback-song").textContent();

    // Click Next.
    await page.getByRole("button", { name: "Next", exact: true }).click();

    // Should advance to a different song.
    await expect(page.locator(".playback-song")).not.toHaveText(
      initialSong!.trim(),
      { timeout: 10000 },
    );
  });

  test("prev goes back after next", async ({ page }) => {
    const initialSong = (await page
      .locator(".playback-song")
      .textContent())!.trim();

    // Advance to next song.
    await page.getByRole("button", { name: "Next", exact: true }).click();
    await expect(page.locator(".playback-song")).not.toHaveText(initialSong, {
      timeout: 10000,
    });

    // Go back.
    await page.getByRole("button", { name: "Prev", exact: true }).click();
    await expect(page.locator(".playback-song")).toHaveText(initialSong, {
      timeout: 10000,
    });
  });

  test("page title updates when playing", async ({ page }) => {
    await page.getByRole("button", { name: "Play" }).click();
    await expect(page.locator(".playback-status .playing")).toBeVisible({
      timeout: 10000,
    });

    // Title should contain the play symbol.
    await expect(page).toHaveTitle(/▶/, { timeout: 5000 });
  });
});
