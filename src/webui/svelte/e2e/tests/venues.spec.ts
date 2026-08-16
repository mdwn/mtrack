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

  test("a venue file that will not parse is named, and the rest still list", async ({
    page,
  }) => {
    // Removing venue-defined groups turned every un-migrated venue file into a
    // parse error, and the loader used to abort the whole directory on the
    // first one — so a single stale file emptied the list with nothing to say
    // why, and re-saving through this very UI was the migration route.
    await page.route("**/api/lighting/venues*", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          venues: {
            "test-venue": { name: "test-venue", fixtures: {} },
          },
          errors: [
            {
              file: "legacy.light",
              error:
                "venue group `wash` is no longer supported. Tag the fixtures instead",
            },
          ],
        }),
      });
    });
    // Re-fetch through the panel's own Refresh rather than reloading the page,
    // which would discard the navigation `beforeEach` performed.
    await page.getByRole("button", { name: "Refresh" }).first().click();

    // The good venue is still listed — the failure did not take it with it.
    await expect(page.locator(".item-name")).toContainText("test-venue");

    // And the bad file is named, with the reason.
    const errors = page.getByTestId("venue-file-errors");
    await expect(errors).toBeVisible();
    await expect(errors).toContainText("legacy.light");
    await expect(errors).toContainText("no longer supported");
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
