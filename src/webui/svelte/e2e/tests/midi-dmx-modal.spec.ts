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

test.describe("MIDI DMX Modal", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
  });

  test("modal displays heading content", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await expect(page.locator(".modal")).toBeVisible();
    // Modal should have some content/heading.
    await expect(page.locator(".modal")).not.toBeEmpty();
  });

  test("modal shows empty state or file list", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    // The modal should show either files or an empty/info state.
    await expect(page.locator(".modal")).toBeVisible();
  });

  test("escape key closes modal", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await expect(page.locator(".modal-overlay")).toBeVisible();
    // Focus the overlay (it has tabindex="-1") before pressing Escape.
    await page.locator(".modal-overlay").focus();
    await page.keyboard.press("Escape");
    await expect(page.locator(".modal-overlay")).not.toBeVisible();
  });

  test("clicking overlay closes modal", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await expect(page.locator(".modal-overlay")).toBeVisible();
    // Click on the overlay (outside the modal content).
    await page.locator(".modal-overlay").click({ position: { x: 5, y: 5 } });
    await expect(page.locator(".modal-overlay")).not.toBeVisible();
  });

  test("Close button closes modal", async ({ page }) => {
    await page.getByRole("button", { name: /midi dmx/i }).click();
    await page.locator(".modal").getByRole("button", { name: "Close" }).click();
    await expect(page.locator(".modal-overlay")).not.toBeVisible();
  });
});
