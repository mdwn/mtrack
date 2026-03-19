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

test.describe("Profile Editor - Lighting/DMX Section", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
  });

  test("shows Enable Lighting checkbox unchecked", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await expect(checkbox).not.toBeChecked();
  });

  test("enabling lighting shows DMX fields", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();

    // Should show DMX universe management
    await expect(page.getByRole("button", { name: "Add" })).toBeVisible();
  });

  test("adding a DMX universe creates a row", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();

    await page.getByRole("button", { name: "Add" }).click();
    await expect(page.locator(".universe-row")).toBeVisible();
  });

  test("DMX universe row is visible after adding", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();

    await page.getByRole("button", { name: "Add" }).click();
    await expect(page.locator(".universe-row")).toBeVisible();
  });

  test("shows OLA port input when enabled", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();

    await expect(page.locator("#dmx-ola-port")).toBeVisible();
  });
});
