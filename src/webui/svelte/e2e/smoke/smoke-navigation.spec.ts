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

test.describe("Smoke: Navigation", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await waitForWebSocket(page);
  });

  test("all nav links are visible and functional", async ({ page }) => {
    // Dashboard (default).
    await expect(page).toHaveTitle(/Dashboard - mtrack/);

    // Songs.
    await page.locator(".nav-link", { hasText: "Songs" }).click();
    await expect(page).toHaveTitle(/Songs - mtrack/);

    // Playlists.
    await page.locator(".nav-link", { hasText: "Playlists" }).click();
    await expect(page).toHaveTitle(/Playlists - mtrack/);

    // Config.
    await page.locator(".nav-link", { hasText: "Config" }).click();
    await expect(page).toHaveTitle(/Config - mtrack/);

    // Status.
    await page.locator(".nav-link", { hasText: "Status" }).click();
    await expect(page).toHaveTitle(/Status - mtrack/);

    // Back to Dashboard.
    await page.locator(".nav-link", { hasText: "Dashboard" }).click();
    await expect(page).toHaveTitle(/Dashboard - mtrack/);
  });

  test("lock toggle is visible", async ({ page }) => {
    const lockToggle = page.locator(".lock-toggle");
    await expect(lockToggle).toBeVisible();
  });

  test("now-playing shows current song in nav", async ({ page }) => {
    const nowPlaying = page.locator(".now-playing-song");
    await expect(nowPlaying).toBeVisible();
    const text = await nowPlaying.textContent();
    expect(text!.trim().length).toBeGreaterThan(0);
  });

  test("WebSocket status indicator shows connected", async ({ page }) => {
    await expect(page.locator(".status-indicator.connected")).toBeVisible();
  });
});
