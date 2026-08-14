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

test.describe("Smoke: Stage View", () => {
  test("stage view page loads without errors", async ({ page }) => {
    await page.goto("/#/stage");

    // Give the page a moment to render.
    await page.waitForTimeout(2000);

    // The page should have rendered something (not blank white).
    // Check that the nav is at least visible (even if stage view is empty).
    await expect(page.locator(".nav-brand")).toBeVisible({ timeout: 10000 });
  });
});
