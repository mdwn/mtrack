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

test.describe("OSC Path Overrides", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Controllers" }).click();

    // Enable controllers and add an OSC controller
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await page.getByRole("button", { name: "Add OSC" }).click();
    await expect(page.locator(".controller-card")).toBeVisible();

    // Show path overrides
    await page.getByRole("button", { name: /show osc path/i }).click();
    await expect(page.locator(".osc-paths")).toBeVisible();
  });

  test("shows play path input", async ({ page }) => {
    await expect(page.locator('[id^="osc-play-"]')).toBeVisible();
  });

  test("shows stop path input", async ({ page }) => {
    await expect(page.locator('[id^="osc-stop-"]')).toBeVisible();
  });

  test("shows next path input", async ({ page }) => {
    await expect(page.locator('[id^="osc-next-"]')).toBeVisible();
  });

  test("shows prev path input", async ({ page }) => {
    await expect(page.locator('[id^="osc-prev-"]')).toBeVisible();
  });

  test("shows playlist path input", async ({ page }) => {
    await expect(page.locator('[id^="osc-playlist-"]').first()).toBeVisible();
  });

  test("hide button collapses path overrides", async ({ page }) => {
    await page.getByRole("button", { name: /hide osc path/i }).click();
    await expect(page.locator(".osc-paths")).not.toBeVisible();
  });

  test("broadcast address Add button creates input", async ({ page }) => {
    // Scroll up to find the broadcast addresses section
    const addBroadcast = page
      .locator(".controller-card")
      .getByRole("button", { name: "Add" });
    await addBroadcast.click();
    await expect(page.locator(".addr-row input")).toBeVisible();
  });
});
