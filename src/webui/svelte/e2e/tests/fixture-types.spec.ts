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

test.describe("Fixture Types Management", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Lighting" }).click();
    await expect(page.locator(".tab.active")).toContainText("Lighting");
    await page.getByRole("button", { name: "Enable Lighting" }).click();
    // Navigate to Fixture Types sub-tab.
    await page.locator(".sub-tab", { hasText: "Fixture Types" }).click();
  });

  test("shows existing fixture type from mock data", async ({ page }) => {
    await expect(page.locator(".item-card")).toBeVisible();
    await expect(page.locator(".item-name")).toContainText("par");
  });

  test("shows channel info for fixture type", async ({ page }) => {
    await expect(page.locator(".item-meta")).toContainText(/channels/);
  });

  test("clicking fixture type opens editor form", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#ft-name")).toBeVisible();
  });

  test("editor shows channel rows for existing fixture", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".channel-row")).toHaveCount(4); // red, green, blue, dimmer
  });

  test("New Fixture Type button opens empty form", async ({ page }) => {
    await page.getByRole("button", { name: "New Fixture Type" }).click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await expect(page.locator("#ft-name")).toHaveValue("");
  });

  test("Add Channel adds a new channel row", async ({ page }) => {
    await page.locator(".item-card").first().click();
    const initialCount = await page.locator(".channel-row").count();
    await page.getByRole("button", { name: "Add Channel" }).click();
    await expect(page.locator(".channel-row")).toHaveCount(initialCount + 1);
  });

  test("removing a channel row decreases count", async ({ page }) => {
    await page.locator(".item-card").first().click();
    const initialCount = await page.locator(".channel-row").count();
    // Click the first X button in a channel row.
    await page.locator(".channel-row").first().locator(".btn-danger").click();
    await expect(page.locator(".channel-row")).toHaveCount(initialCount - 1);
  });

  test("saving fixture type calls PUT API", async ({ page }) => {
    let saveCalled = false;
    await page.route("**/api/lighting/fixture-types/*", async (route) => {
      if (route.request().method() === "PUT") {
        saveCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "saved" }),
        });
      } else {
        await route.continue();
      }
    });

    await page.locator(".item-card").first().click();
    await page
      .locator(".editor-form")
      .getByRole("button", { name: "Save" })
      .click();
    expect(saveCalled).toBe(true);
  });

  test("cancel button closes editor form", async ({ page }) => {
    await page.locator(".item-card").first().click();
    await expect(page.locator(".editor-form")).toBeVisible();
    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.locator(".editor-form")).not.toBeVisible();
  });
});
