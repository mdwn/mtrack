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

test.describe("Profile Editor - MIDI Section", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await expect(page.locator(".tab.active")).toContainText("MIDI");
  });

  test("shows Enable MIDI checkbox unchecked", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await expect(checkbox).not.toBeChecked();
  });

  test("shows panel empty message when disabled", async ({ page }) => {
    await expect(page.locator(".panel-empty")).toBeVisible();
  });

  test("enabling MIDI shows device input", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await expect(page.locator("#midi-device")).toBeVisible();
  });

  test("enabling MIDI shows delay input", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await expect(page.locator("#midi-delay")).toBeVisible();
  });

  test("enabling MIDI shows beat clock checkbox", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await expect(page.locator("#midi-beat-clock")).toBeVisible();
  });

  test("enabling MIDI shows Refresh button", async ({ page }) => {
    const checkbox = page.locator(".enable-toggle input[type='checkbox']");
    await checkbox.check();
    await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
  });
});
