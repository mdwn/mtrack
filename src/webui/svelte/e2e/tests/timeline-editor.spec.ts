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

test.describe("Lighting Timeline Editor", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs/Test%20Song%20Alpha/lighting");
    await expect(page.locator(".tab.active")).toContainText("Lighting");
  });

  test("Timeline tab is active by default", async ({ page }) => {
    await expect(
      page.locator(".tab-btn", { hasText: "Timeline" }),
    ).toBeVisible();
    // Timeline content should be showing (not Raw DSL).
    await expect(page.locator(".raw-textarea")).not.toBeVisible();
  });

  test("shows lighting show selector", async ({ page }) => {
    // The show selector or file list should be visible.
    await expect(page.locator(".lighting-section")).toBeVisible();
  });

  test("switching to Raw DSL and back preserves state", async ({ page }) => {
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await expect(page.locator(".raw-textarea")).toBeVisible();

    await page.locator(".tab-btn", { hasText: "Timeline" }).click();
    await expect(page.locator(".raw-textarea")).not.toBeVisible();
  });

  test("Raw DSL tab has Validate button", async ({ page }) => {
    await page.locator(".tab-btn", { hasText: "Raw DSL" }).click();
    await expect(page.getByRole("button", { name: "Validate" })).toBeVisible();
  });

  test("+ DSL button is visible", async ({ page }) => {
    await expect(page.getByRole("button", { name: /\+ DSL/i })).toBeVisible();
  });

  test("MIDI DMX button is visible", async ({ page }) => {
    await expect(page.getByRole("button", { name: /midi dmx/i })).toBeVisible();
  });
});
