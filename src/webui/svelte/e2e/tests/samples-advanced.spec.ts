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

test.describe("Samples - Advanced Features", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
  });

  test("adding multiple samples creates multiple cards", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    await page.getByRole("button", { name: "Add Sample" }).click();
    await expect(page.locator(".sample-card")).toHaveCount(2);
  });

  test("removing a sample removes its card", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    await page.getByRole("button", { name: "Add Sample" }).click();
    await expect(page.locator(".sample-card")).toHaveCount(2);

    // Remove the first sample — requires confirmation now.
    await page
      .locator(".sample-card")
      .first()
      .getByRole("button", { name: "Remove" })
      .click();
    await page.getByRole("button", { name: "Confirm" }).click();
    await expect(page.locator(".sample-card")).toHaveCount(1);
  });

  test("sample card shows default name", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    // Should have some default name text in the header.
    await expect(page.locator(".sample-card .sample-title")).toBeVisible();
  });

  test("new sample starts in edit mode", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    await expect(page.locator(".sample-card")).toBeVisible();
    // New samples start with the name input focused.
    await expect(page.locator(".sample-card .name-input")).toBeVisible();
  });

  test("pressing Enter on name exits edit mode and shows name text", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    const input = page.locator(".sample-card .name-input");
    await expect(input).toBeVisible();
    await input.fill("my-kick");
    await input.press("Enter");
    await expect(page.locator(".sample-card .name-text").first()).toContainText(
      "my-kick",
    );
  });

  test("expanding advanced settings shows dropdowns", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    // Expand the sample body first (click header to toggle).
    const card = page.locator(".sample-card").first();
    await expect(card.locator(".sample-body")).toBeVisible();

    // Click advanced toggle.
    await card.locator(".advanced-toggle").click();
    await expect(card.locator(".advanced-body")).toBeVisible();
  });

  test("advanced settings shows release mode select", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    const card = page.locator(".sample-card").first();
    await card.locator(".advanced-toggle").click();
    // Look for a select element related to release.
    const releaseSelect = card.locator("select").first();
    await expect(releaseSelect).toBeVisible();
  });

  test("collapse icon toggles on click", async ({ page }) => {
    await page.getByRole("button", { name: "Add Sample" }).click();
    const card = page.locator(".sample-card").first();
    const icon = card.locator(".collapse-icon");

    // Initially expanded (shows "-").
    await expect(icon).toContainText("-");

    // Click header to collapse.
    await card.locator(".sample-header").click();
    await expect(icon).toContainText("+");
  });
});
