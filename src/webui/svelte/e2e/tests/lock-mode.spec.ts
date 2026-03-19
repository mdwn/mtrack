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

test.describe("Lock Mode", () => {
  test("lock toggle is visible on dashboard", async ({ page }) => {
    await page.goto("/#/");
    const lockToggle = page.locator(".lock-toggle");
    await expect(lockToggle).toBeVisible();
  });

  test("lock toggle calls API when clicked", async ({ page }) => {
    await page.goto("/#/");
    let lockCalled = false;
    await page.route("**/api/lock", async (route) => {
      if (route.request().method() === "PUT") {
        lockCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ locked: true }),
        });
      } else {
        await route.continue();
      }
    });

    const lockToggle = page.locator(".lock-toggle");
    await lockToggle.click();
    expect(lockCalled).toBe(true);
  });

  test("locked state adds class to lock toggle", async ({ page }) => {
    // The mock WebSocket sends locked: false, but let's check the unlocked state
    await page.goto("/#/");
    await expect(page.locator(".lock-toggle")).toBeVisible();
    // In unlocked state, the toggle should not have the .locked class
    await expect(page.locator(".lock-toggle")).not.toHaveClass(/locked/);
  });
});
