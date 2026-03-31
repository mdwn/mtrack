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

test.describe("Venues Management", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await page.getByRole("button", { name: "Enable Lighting" }).click();
    // Navigate to Venues sub-tab.
    await page.locator(".sub-tab", { hasText: "Venues" }).click();
  });

  test("shows existing venue from mock data", async ({ page }) => {
    await expect(page.locator(".item-card")).toBeVisible();
    await expect(page.locator(".item-name")).toContainText("test-venue");
  });

  test("venue card shows metadata", async ({ page }) => {
    await expect(page.locator(".item-meta").first()).toBeVisible();
  });

  test("New Venue button opens empty form", async ({ page }) => {
    await page.getByRole("button", { name: "New Venue" }).click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#venue-name")).toHaveValue("");
  });

  test("clicking venue opens editor form", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#venue-name")).toBeVisible();
  });

  test("Add Fixture adds a fixture row", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    const initialCount = await page.locator(".venue-fixture-card").count();
    await page.getByRole("button", { name: "Add Fixture" }).click();
    await expect(page.locator(".venue-fixture-card")).toHaveCount(
      initialCount + 1,
    );
  });

  test("saving venue calls PUT API", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();

    const requestPromise = page.waitForRequest(
      (req) =>
        req.url().includes("/api/lighting/venues/") && req.method() === "PUT",
    );
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    await requestPromise;
  });

  test("cancel button closes venue editor", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".editor-form")).not.toBeVisible();
  });
});
