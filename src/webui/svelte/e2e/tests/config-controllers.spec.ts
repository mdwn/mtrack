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

test.describe("Profile Editor - Controllers", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/#/config");
    await page.locator(".profile-row", { hasText: "test-host" }).click();
    await expect(page.getByRole("button", { name: "Back" })).toBeVisible();
    // Click the Controllers tab, then enable the section
    await page.locator(".tab", { hasText: "Controllers" }).click();
    await expect(page.locator(".tab.active")).toContainText("Controllers");
    await page.getByRole("button", { name: "Enable Controllers" }).click();
  });

  test("controllers tab shows Add buttons", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Add gRPC" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Add OSC" })).toBeVisible();
  });

  test("adding gRPC controller shows port field", async ({ page }) => {
    await page.getByRole("button", { name: "Add gRPC" }).click();
    await expect(page.locator(".controller-card")).toBeVisible();
    await expect(page.locator(".controller-kind")).toContainText(/grpc/i);
    const portInput = page.locator('input[placeholder="43234"]');
    await expect(portInput).toBeVisible();
  });

  test("adding OSC controller shows port field", async ({ page }) => {
    await page.getByRole("button", { name: "Add OSC" }).click();
    await expect(page.locator(".controller-card")).toBeVisible();
    await expect(page.locator(".controller-kind")).toContainText(/osc/i);
    const portInput = page.locator('input[placeholder="43235"]');
    await expect(portInput).toBeVisible();
  });

  test("removing controller removes card", async ({ page }) => {
    await page.getByRole("button", { name: "Add gRPC" }).click();
    await expect(page.locator(".controller-card")).toHaveCount(1);

    await page
      .locator(".controller-card")
      .getByRole("button", { name: "Remove" })
      .click();
    await expect(page.locator(".controller-card")).toHaveCount(0);
  });

  test("OSC controller shows path overrides toggle", async ({ page }) => {
    await page.getByRole("button", { name: "Add OSC" }).click();
    await expect(
      page.getByRole("button", { name: /show osc path/i }),
    ).toBeVisible();
  });

  test("toggling OSC path overrides shows path inputs", async ({ page }) => {
    await page.getByRole("button", { name: "Add OSC" }).click();
    await page.getByRole("button", { name: /show osc path/i }).click();
    await expect(page.locator(".osc-paths")).toBeVisible();
  });
});
