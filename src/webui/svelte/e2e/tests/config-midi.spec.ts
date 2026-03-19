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
    // Click the MIDI tab, then enable the section
    await page.locator(".tab", { hasText: "MIDI" }).click();
    await expect(page.locator(".tab.active")).toContainText("MIDI");
    await page.getByRole("button", { name: "Enable MIDI" }).click();
  });

  test("MIDI tab shows device input", async ({ page }) => {
    await expect(page.locator("#midi-device")).toBeVisible();
  });

  test("MIDI tab shows delay input", async ({ page }) => {
    await expect(page.locator("#midi-delay")).toBeVisible();
  });

  test("MIDI tab shows beat clock checkbox", async ({ page }) => {
    await expect(page.locator("#midi-beat-clock")).toBeVisible();
  });

  test("MIDI tab shows Refresh button", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Refresh" })).toBeVisible();
  });
});
