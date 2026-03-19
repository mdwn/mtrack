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

test.describe("Song Deletion", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/songs");
    await expect(
      page.locator(".song-row", { hasText: "Test Song Alpha" }),
    ).toBeVisible();
  });

  test("song row shows delete button on hover", async ({ page }) => {
    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await expect(songRow.locator(".song-delete")).toBeVisible();
  });

  test("delete calls API", async ({ page }) => {
    let deleteCalled = false;
    await page.route("**/api/songs/Test%20Song%20Alpha", async (route) => {
      if (route.request().method() === "DELETE") {
        deleteCalled = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ status: "deleted" }),
        });
      } else {
        await route.continue();
      }
    });

    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await songRow.locator(".song-delete").click();

    // Confirm via custom dialog
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Confirm" }).click();

    expect(deleteCalled).toBe(true);
  });

  test("cancelling delete dialog does not call API", async ({ page }) => {
    let deleteCalled = false;
    await page.route("**/api/songs/Test%20Song%20Alpha", async (route) => {
      if (route.request().method() === "DELETE") {
        deleteCalled = true;
      }
      await route.continue();
    });

    const songRow = page.locator(".song-row", { hasText: "Test Song Alpha" });
    await songRow.hover();
    await songRow.locator(".song-delete").click();

    // Cancel via custom dialog
    const dialog = page.locator('[role="dialog"]');
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel" }).click();

    expect(deleteCalled).toBe(false);
  });
});
