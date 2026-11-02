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

// A browser without WebAssembly: the tempo map (the player's, compiled to
// WebAssembly) cannot load, so the views that lay bars out say so, and the
// rest of the UI works as ever.

import { test, expect } from "@playwright/test";
import { DashboardPage } from "../pages/dashboard.page.js";

test.describe("Without WebAssembly", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      delete (globalThis as { WebAssembly?: unknown }).WebAssembly;
    });
  });

  test("the dashboard and the playback controls work", async ({ page }) => {
    const dashboard = new DashboardPage(page);
    await dashboard.goto();
    await expect(dashboard.grid).toBeVisible();
    await expect(dashboard.songName).toHaveText("Test Song Alpha");
    await expect(dashboard.playButton).toBeVisible();
    await expect(dashboard.playButton).toBeEnabled();
  });

  test("the lighting timeline says why it cannot open", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    const alert = page.getByTestId("tempo-map-unavailable");
    await expect(alert).toBeVisible();
    await expect(alert).toContainText("WebAssembly");
    await expect(page.locator(".timeline-editor")).toHaveCount(0);
    // The rest of the song page still works: the raw DSL opens.
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await expect(page.locator("textarea").first()).toBeVisible();
  });

  test("the section timeline says why it cannot open", async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Gamma");
    await page.locator(".tab", { hasText: "Timeline" }).click();
    await expect(page.getByTestId("tempo-map-unavailable")).toBeVisible();
    await expect(page.locator(".section-block")).toHaveCount(0);
  });
});
