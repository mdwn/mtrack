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

test.describe("Tempo Editor", () => {
  test("lighting tab loads for song with beat grid", async ({ page }) => {
    // Test Song Beta has a beat grid and sections.
    await page.goto("/#/songs/Test%20Song%20Beta/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
  });

  test("lighting tab loads for song without beat grid", async ({ page }) => {
    // Test Song Alpha has no beat grid.
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await expect(page.locator(".lighting-section")).toBeVisible();
    // Tempo info badge should not show specific BPM info.
    await expect(page.locator(".tempo-info")).not.toBeVisible();
  });
});
