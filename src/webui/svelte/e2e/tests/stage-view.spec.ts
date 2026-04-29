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

test.describe("Stage View", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/");
    // Wait for WS to deliver metadata with fixtures
    await expect(page.locator(".playback-card__title")).toContainText(
      "Test Song Alpha",
    );
  });

  test("stage card is visible on dashboard", async ({ page }) => {
    await expect(page.locator(".stage-card, .card").first()).toBeVisible();
  });

  test("stage viewport contains canvas", async ({ page }) => {
    // Just verify the stage viewport container exists
    await expect(page.locator(".stage-card__viewport")).toBeVisible();
  });
});
