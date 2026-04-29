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
    await expect(page.locator(".topnav__conn")).toBeVisible();
    await expect(page.locator(".topnav__conn")).not.toHaveClass(
      /topnav__conn--off/,
    );
  });

  test("playback store populates from WS message", async ({ page }) => {
    await page.goto("/#/");
    // Playback data comes from WebSocket, not REST
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
    await expect(page.locator(".playback-card__state")).toContainText(/stopped/i);
  });

  test("waveform data loads from WS message", async ({ page }) => {
    await page.goto("/#/");
    // Waveform data arrives via WebSocket — verify tracks card has track rows
    // (canvas rendering depends on waveform data being present)
    await expect(page.locator(".tracks-card__row").first()).toBeVisible();
    await expect(page.locator(".tracks-card__waveform").first()).toBeVisible();
  });

  test("playlist songs populate from WS message", async ({ page }) => {
    await page.goto("/#/");
    const songs = page.locator(".playlist-card__list li");
    await expect(songs).toHaveCount(2);
  });

  test("available playlists populate from WS message", async ({ page }) => {
    await page.goto("/#/");
    const options = page.locator(".playlist-card__select option");
    await expect(options).toHaveCount(2);
  });

  test("track info populates from WS message", async ({ page }) => {
    await page.goto("/#/");
    const tracks = page.locator(".tracks-card__row");
    await expect(tracks).toHaveCount(3);

    // Track names
    await expect(tracks.nth(0)).toContainText("kick");
    await expect(tracks.nth(1)).toContainText("snare");
    await expect(tracks.nth(2)).toContainText("bass");
  });

  test("track rows show output channel info", async ({ page }) => {
    await page.goto("/#/");
    const firstTrack = page.locator(".tracks-card__row").first();
    await expect(firstTrack.locator(".tracks-card__channels")).toBeVisible();
  });

  test("disconnect banner is not visible when connected", async ({ page }) => {
    await page.goto("/#/");
    await expect(page.locator(".topnav__conn")).toBeVisible();
    await expect(page.locator(".topnav__conn")).not.toHaveClass(
      /topnav__conn--off/,
    );
    await expect(page.locator(".disconnect-banner")).not.toBeVisible();
  });
});
