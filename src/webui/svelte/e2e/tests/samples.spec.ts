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

test.describe("Samples Section", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
  });

  test("shows Samples heading", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Samples" })).toBeVisible();
  });

  test("shows empty state when no samples configured", async ({ page }) => {
    // The mock config has samples: {} so should show empty state
    await expect(page.getByText("No samples configured")).toBeVisible();
  });

  test("shows Add Sample button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: "Add Sample" }),
    ).toBeVisible();
  });

  test("clicking Add Sample creates a new sample card", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    // Should show a sample card with default name
    await expect(page.locator(".sample-card")).toBeVisible();
  });

  test("save samples button starts disabled", async ({ page }) => {
    const saveBtn = page.getByRole("button", { name: "Save Samples" });
    await expect(saveBtn).toBeDisabled();
  });

  test("save samples enables after adding a sample", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    const saveBtn = page.getByRole("button", { name: "Save Samples" });
    await expect(saveBtn).toBeEnabled();
  });

  test("removing a sample requires confirmation dialog", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    await expect(page.locator(".sample-card")).toBeVisible();

    // Click Remove on the sample card
    await page
      .locator(".sample-card")
      .getByRole("button", { name: "Remove" })
      .click();

    // Confirmation dialog should appear
    await expect(page.locator(".dialog-overlay")).toBeVisible();
    await expect(page.locator(".dialog-message")).toContainText(
      "Remove sample",
    );

    // Cancel should keep the sample
    await page
      .locator(".dialog-overlay")
      .getByRole("button", { name: "Cancel" })
      .click();
    await expect(page.locator(".dialog-overlay")).not.toBeVisible();
    await expect(page.locator(".sample-card")).toBeVisible();

    // Click Remove again, then confirm
    await page
      .locator(".sample-card")
      .getByRole("button", { name: "Remove" })
      .click();
    await expect(page.locator(".dialog-overlay")).toBeVisible();
    await page
      .locator(".dialog-overlay")
      .getByRole("button", { name: "Confirm" })
      .click();
    await expect(page.locator(".dialog-overlay")).not.toBeVisible();
    await expect(page.locator(".sample-card")).not.toBeVisible();
  });

  test("save samples calls API", async ({ page }) => {
    let saveCalled = false;
    await page.route("**/api/config/samples", async (route) => {
      if (route.request().method() === "PUT") {
        saveCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ yaml: "", checksum: "new-checksum" }),
        });
      } else {
        await route.continue();
      }
    });

    await page.getByRole("button", { name: "Add Sample" }).click();
    await page.getByRole("button", { name: "Save Samples" }).click();
    expect(saveCalled).toBe(true);
  });
});
