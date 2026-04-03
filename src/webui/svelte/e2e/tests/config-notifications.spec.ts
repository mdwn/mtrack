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

test.describe("Profile Editor - Notifications", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    await page.locator(".tab", { hasText: "Notifications" }).click();
    await expect(page.locator(".tab.active")).toContainText("Notifications");
  });

  test("shows enable button when not configured", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /Enable Notifications/i }),
    ).toBeVisible();
  });

  test("enabling shows notification fields", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Notifications/i }).click();
    await expect(page.locator("#notif-loop_armed")).toBeVisible();
    await expect(page.locator("#notif-break_requested")).toBeVisible();
    await expect(page.locator("#notif-loop_exited")).toBeVisible();
    await expect(page.locator("#notif-section_entering")).toBeVisible();
  });

  test("shows per-section overrides area", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Notifications/i }).click();
    await expect(page.getByText(/Per-Section Overrides/i)).toBeVisible();
  });

  test("add button creates section override row", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Notifications/i }).click();
    const addBtn = page
      .locator(".sections-area")
      .getByRole("button", { name: "Add" });
    await addBtn.click();
    await expect(page.locator(".section-row")).toHaveCount(1);
  });

  test("remove button deletes section override row", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Notifications/i }).click();
    const addBtn = page
      .locator(".sections-area")
      .getByRole("button", { name: "Add" });
    await addBtn.click();
    await expect(page.locator(".section-row")).toHaveCount(1);
    await page
      .locator(".section-row")
      .getByRole("button", { name: "X" })
      .click();
    await expect(page.locator(".section-row")).toHaveCount(0);
  });

  test("shows browse buttons on notification fields", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Notifications/i }).click();
    await expect(page.locator(".browse-btn").first()).toBeVisible();
  });

  test("shows file upload drop zone", async ({ page }) => {
    await page.getByRole("button", { name: /Enable Notifications/i }).click();
    await expect(page.locator(".drop-zone")).toBeVisible();
  });
});
